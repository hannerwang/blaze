// Copyright 2022 The Blaze Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::shuffle::{
    evaluate_hashes, evaluate_partition_ids, ShuffleRepartitioner, ShuffleSpill,
};
use crate::spill::Spill;
use arrow::array::*;
use arrow::compute;
use arrow::compute::TakeOptions;
use arrow::datatypes::SchemaRef;
use arrow::error::Result as ArrowResult;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use bytesize::ByteSize;
use datafusion::common::{DataFusionError, Result};
use datafusion::execution::context::TaskContext;
use datafusion::execution::memory_manager::ConsumerType;
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::execution::{MemoryConsumer, MemoryConsumerId, MemoryManager};
use datafusion::physical_plan::coalesce_batches::concat_batches;
use datafusion::physical_plan::metrics::BaselineMetrics;
use datafusion::physical_plan::Partitioning;
use datafusion_ext_commons::io::write_one_batch;
use datafusion_ext_commons::loser_tree::LoserTree;
use derivative::Derivative;
use futures::lock::Mutex;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, Write};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;
use voracious_radix_sort::{RadixSort, Radixable};

pub struct SortShuffleRepartitioner {
    memory_consumer_id: MemoryConsumerId,
    output_data_file: String,
    output_index_file: String,
    schema: SchemaRef,
    buffered_batches: Mutex<Vec<RecordBatch>>,
    buffered_mem_size: AtomicUsize,
    spills: Mutex<Vec<ShuffleSpill>>,
    partitioning: Partitioning,
    num_output_partitions: usize,
    runtime: Arc<RuntimeEnv>,
    batch_size: usize,
    metrics: BaselineMetrics,
}

impl Debug for SortShuffleRepartitioner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortShuffleRepartitioner")
            .field("id", &self.id())
            .field("memory_used", &self.mem_used())
            .field("spilled_bytes", &self.spilled_bytes())
            .field("spilled_count", &self.spill_count())
            .finish()
    }
}

impl SortShuffleRepartitioner {
    pub fn new(
        partition_id: usize,
        output_data_file: String,
        output_index_file: String,
        schema: SchemaRef,
        partitioning: Partitioning,
        metrics: BaselineMetrics,
        context: Arc<TaskContext>,
    ) -> Self {
        let num_output_partitions = partitioning.partition_count();
        let runtime = context.runtime_env();
        let batch_size = context.session_config().batch_size();
        let repartitioner = Self {
            memory_consumer_id: MemoryConsumerId::new(partition_id),
            output_data_file,
            output_index_file,
            schema,
            buffered_batches: Mutex::default(),
            buffered_mem_size: AtomicUsize::new(0),
            spills: Mutex::default(),
            partitioning,
            num_output_partitions,
            runtime,
            batch_size,
            metrics,
        };
        repartitioner.runtime.register_requester(repartitioner.id());
        repartitioner
    }

    async fn spill_buffered_to_l1(&self) -> Result<ShuffleSpill> {
        let mut buffered_batches = self.buffered_batches.lock().await;

        // combine all buffered batches
        let num_output_partitions = self.num_output_partitions;
        let num_buffered_rows = buffered_batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>();
        let batch = concat_batches(
            &self.schema,
            &std::mem::take::<Vec<RecordBatch>>(&mut buffered_batches),
            num_buffered_rows,
        )?;

        let hashes = evaluate_hashes(&self.partitioning, &batch)?;
        let partition_ids = evaluate_partition_ids(&hashes, num_output_partitions);

        // compute partition ids and sorted indices by counting sort
        let mut pi_vec = hashes
            .into_iter()
            .zip(partition_ids.into_iter())
            .enumerate()
            .map(|(i, (hash, partition_id))| PI {
                partition_id,
                hash,
                index: i as u32,
            })
            .collect::<Vec<_>>();
        pi_vec.voracious_sort();

        // write to in-mem spill
        let mut cur_partition_id = 0;
        let mut cur_slice_start = 0;
        let mut cur_spill_frozen = vec![];
        let mut cur_spill_offsets = vec![0];
        let mut frozen_cursor = Cursor::new(&mut cur_spill_frozen);

        macro_rules! write_sub_batch {
            ($range:expr) => {{
                let sub_pi_vec = &pi_vec[$range];
                let sub_indices =
                    UInt32Array::from_iter_values(sub_pi_vec.iter().map(|pi| pi.index));

                let sub_batch = RecordBatch::try_new(
                    self.schema.clone(),
                    batch
                        .columns()
                        .iter()
                        .map(|column| {
                            compute::take(
                                column,
                                &sub_indices,
                                Some(TakeOptions {
                                    check_bounds: false,
                                }),
                            )
                        })
                        .collect::<ArrowResult<Vec<_>>>()?,
                )?;
                write_one_batch(&sub_batch, &mut frozen_cursor, true)?;
            }};
        }

        // write sorted data into in-mem spill
        for cur_offset in 0..pi_vec.len() {
            if pi_vec[cur_offset].partition_id > cur_partition_id
                || cur_offset - cur_slice_start >= self.batch_size
            {
                if cur_slice_start < cur_offset {
                    write_sub_batch!(cur_slice_start..cur_offset);
                    cur_slice_start = cur_offset;
                }

                while pi_vec[cur_offset].partition_id > cur_partition_id {
                    cur_spill_offsets.push(frozen_cursor.stream_position()?);
                    cur_partition_id += 1;
                }
            }
        }
        if cur_slice_start < pi_vec.len() {
            write_sub_batch!(cur_slice_start..);
        }

        // add one extra offset at last to ease partition length computation
        cur_spill_offsets
            .resize(num_output_partitions + 1, cur_spill_frozen.len() as u64);
        self.buffered_mem_size.store(0, SeqCst);
        Ok(ShuffleSpill {
            spill: Spill::new_l1(cur_spill_frozen),
            offsets: cur_spill_offsets,
        })
    }

    fn spilled_bytes(&self) -> usize {
        self.metrics.spilled_bytes().value()
    }

    fn spill_count(&self) -> usize {
        self.metrics.spill_count().value()
    }
}

#[async_trait]
impl MemoryConsumer for SortShuffleRepartitioner {
    fn name(&self) -> String {
        "SortShuffleRepartitioner".to_string()
    }

    fn id(&self) -> &MemoryConsumerId {
        &self.memory_consumer_id
    }

    fn memory_manager(&self) -> Arc<MemoryManager> {
        self.runtime.memory_manager.clone()
    }

    fn type_(&self) -> &ConsumerType {
        &ConsumerType::Requesting
    }

    async fn spill(&self) -> Result<usize> {
        let current_used = self.mem_used();
        log::info!(
            "sort repartitioner start spilling, used={}, {}",
            ByteSize(current_used as u64),
            self.memory_manager(),
        );

        let mut spills = self.spills.lock().await;
        let mut freed = 0isize;

        // first try spill current buffered batches to in-mem spills
        let buffered_mem_size = self.buffered_mem_size.load(SeqCst);
        freed += buffered_mem_size as isize;
        let in_mem_spill = self.spill_buffered_to_l1().await?;
        freed -= in_mem_spill.spill.offheap_mem_size() as isize;
        log::info!(
            "sort repartitioner spilled into memory, freed={}",
            ByteSize(freed as u64),
        );
        spills.push(in_mem_spill);

        // move L1 into L2/L3 if necessary
        while freed < current_used as isize / 2 {
            let pop_index = spills
                .iter()
                .enumerate()
                .max_by_key(|(_, spill)| spill.spill.offheap_mem_size())
                .unwrap()
                .0;
            let pop_spill = spills.remove(pop_index);
            let pop_spill_mem_size = pop_spill.spill.offheap_mem_size();
            freed += pop_spill_mem_size as isize;

            let spill = match pop_spill.spill.to_l2() {
                Ok(spill) => {
                    log::info!(
                        "sort repartitioner spilled into L2: size={}",
                        ByteSize(pop_spill_mem_size as u64),
                    );
                    ShuffleSpill {
                        spill,
                        offsets: pop_spill.offsets,
                    }
                }
                Err(DataFusionError::ResourcesExhausted(..)) => {
                    let spill = pop_spill.spill.to_l3(&self.runtime.disk_manager)?;
                    log::info!(
                        "sort repartitioner spilled into L3: size={}",
                        ByteSize(pop_spill_mem_size as u64),
                    );
                    self.metrics.record_spill(pop_spill_mem_size);
                    ShuffleSpill {
                        spill,
                        offsets: pop_spill.offsets,
                    }
                }
                Err(err) => {
                    return Err(err);
                }
            };
            spills.push(spill);
        }
        let freed = freed as usize;
        self.metrics.mem_used().sub(freed);
        Ok(freed)
    }

    fn mem_used(&self) -> usize {
        self.metrics.mem_used().value()
    }
}

#[async_trait]
impl ShuffleRepartitioner for SortShuffleRepartitioner {
    fn name(&self) -> &str {
        "sort repartitioner"
    }

    async fn insert_batch(&self, input: RecordBatch) -> Result<()> {
        // first grow memory usage of cur batch
        // NOTE:
        //  when spilling, buffered batches are first spilled into memory.
        //  batches and compressed frozen bytes are both in memory during
        //  spill. to avoid memory overflow, we aquire more memory than
        //  the actual bytes size.
        let mem_increase_actual = input.get_array_memory_size();
        let mem_increase = mem_increase_actual * 2;

        self.try_grow(mem_increase).await?;
        self.metrics.mem_used().add(mem_increase);

        let mut buffered_batches = self.buffered_batches.lock().await;
        self.buffered_mem_size.fetch_add(mem_increase, SeqCst);
        buffered_batches.push(input);
        Ok(())
    }

    async fn shuffle_write(&self) -> Result<()> {
        let mut spills = self.spills.lock().await;

        // spill all buffered batches
        if self.buffered_mem_size.load(SeqCst) > 0 {
            spills.push(self.spill_buffered_to_l1().await?);
        }

        // define spill cursor. partial-ord is reversed because we
        // need to find mininum using a binary heap
        #[derive(Derivative)]
        #[derivative(PartialOrd, PartialEq, Ord, Eq)]
        struct SpillCursor {
            cur: usize,

            #[derivative(PartialOrd = "ignore")]
            #[derivative(PartialEq = "ignore")]
            #[derivative(Ord = "ignore")]
            reader: Box<dyn Read + Send>,

            #[derivative(PartialOrd = "ignore")]
            #[derivative(PartialEq = "ignore")]
            #[derivative(Ord = "ignore")]
            offsets: Vec<u64>,
        }
        impl SpillCursor {
            fn skip_empty_partitions(&mut self) {
                let offsets = &self.offsets;
                while !self.finished() && offsets[self.cur + 1] == offsets[self.cur] {
                    self.cur += 1;
                }
            }

            fn finished(&self) -> bool {
                self.cur + 1 >= self.offsets.len()
            }
        }

        // use loser tree to select partitions from spills
        let mut spills: LoserTree<SpillCursor> = LoserTree::new_by(
            std::mem::take(&mut *spills)
                .into_iter()
                .map(|spill| {
                    let mut cursor = SpillCursor {
                        cur: 0,
                        reader: spill.spill.into_reader(),
                        offsets: spill.offsets,
                    };
                    cursor.skip_empty_partitions();
                    cursor
                })
                .filter(|spill| !spill.finished())
                .collect(),
            |c1: &SpillCursor, c2: &SpillCursor| match (c1, c2) {
                (c1, _2) if c1.finished() => false,
                (_1, c2) if c2.finished() => true,
                (c1, c2) => c1 < c2,
            },
        );

        let data_file = self.output_data_file.clone();
        let index_file = self.output_index_file.clone();

        let num_output_partitions = self.num_output_partitions;
        let mut offsets = vec![0];
        let mut output_data = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(data_file)?;
        let mut cur_partition_id = 0;

        // append partition in each spills
        if spills.len() > 0 {
            loop {
                let mut min_spill = spills.peek_mut();
                if min_spill.finished() {
                    break;
                }

                while cur_partition_id < min_spill.cur {
                    offsets.push(output_data.stream_position()?);
                    cur_partition_id += 1;
                }
                let (spill_offset_start, spill_offset_end) = (
                    min_spill.offsets[cur_partition_id],
                    min_spill.offsets[cur_partition_id + 1],
                );

                let spill_range = spill_offset_start as usize..spill_offset_end as usize;
                let reader = &mut min_spill.reader;
                std::io::copy(
                    &mut reader.take(spill_range.len() as u64),
                    &mut output_data,
                )?;

                // forward partition id in min_spill
                min_spill.cur += 1;
                min_spill.skip_empty_partitions();
            }
        }
        output_data.flush()?;

        // add one extra offset at last to ease partition length computation
        offsets.resize(num_output_partitions + 1, output_data.stream_position()?);

        let mut output_index = File::create(index_file)?;
        for offset in offsets {
            output_index.write_all(&(offset as i64).to_le_bytes()[..])?;
        }
        output_index.flush()?;

        let used = self.metrics.mem_used().set(0);
        self.shrink(used);
        Ok(())
    }
}

impl Drop for SortShuffleRepartitioner {
    fn drop(&mut self) {
        self.runtime.drop_consumer(self.id(), self.mem_used());
    }
}

#[derive(Derivative)]
#[derivative(Clone, Copy, Default, PartialOrd, PartialEq, Ord, Eq)]
struct PI {
    partition_id: u32,
    hash: u32,

    #[derivative(PartialOrd = "ignore")]
    #[derivative(PartialEq = "ignore")]
    #[derivative(Ord = "ignore")]
    index: u32,
}
impl Radixable<u64> for PI {
    type Key = u64;

    fn key(&self) -> Self::Key {
        (self.partition_id as u64) << 32 | self.hash as u64
    }
}
