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

pub use datafusion;
pub use jni;
pub use jni::errors::Result as JniResult;
pub use jni::objects::JClass;
pub use jni::objects::JMethodID;
pub use jni::objects::JObject;
pub use jni::objects::JStaticMethodID;
pub use jni::objects::JValue;
pub use jni::signature::Primitive;
pub use jni::signature::ReturnType;
pub use jni::sys::jvalue;
pub use jni::JNIEnv;
pub use jni::JavaVM;
pub use paste::paste;

use crate::ResultExt;
use once_cell::sync::OnceCell;

thread_local! {
    pub static THREAD_JNIENV: once_cell::unsync::Lazy<JNIEnv<'static>> =
        once_cell::unsync::Lazy::new(|| {
            let jvm = &JavaClasses::get().jvm;
            let env = jvm.attach_current_thread_permanently().unwrap_or_fatal();
            env.call_static_method_unchecked(
                JavaClasses::get().cJniBridge.class,
                JavaClasses::get().cJniBridge.method_setContextClassLoader,
                JavaClasses::get().cJniBridge.method_setContextClassLoader_ret.clone(),
                &[jni::sys::jvalue::from(jni::objects::JValue::from(JavaClasses::get().classloader))]
            ).unwrap_or_fatal();
            env
        });
}

#[macro_export]
macro_rules! jvalues {
    ($($args:expr,)* $(,)?) => {{
        &[$($crate::jni_bridge::JValue::from($args)),*] as &[$crate::jni_bridge::JValue]
    }}
}

#[macro_export]
macro_rules! jvalues_sys {
    ($($args:expr,)* $(,)?) => {{
        &[$($crate::jni_bridge::jvalue::from($crate::jni_bridge::JValue::from($args))),*]
            as &[$crate::jni_bridge::jvalue]
    }}
}

#[macro_export]
macro_rules! jni_map_error_with_env {
    ($env:expr, $result:expr) => {{
        match $result {
            Ok(result) => $crate::jni_bridge::datafusion::error::Result::Ok(result),
            Err($crate::jni_bridge::jni::errors::Error::JavaException) => {
                let ex = $env.exception_occurred().unwrap();
                $env.exception_describe().unwrap();
                $env.exception_clear().unwrap();
                let message_obj = $env
                    .call_method_unchecked(
                        ex,
                        $crate::jni_bridge::JavaClasses::get()
                            .cJavaThrowable
                            .method_getMessage,
                        $crate::jni_bridge::JavaClasses::get()
                            .cJavaThrowable
                            .method_getMessage_ret
                            .clone(),
                        &[],
                    )
                    .unwrap()
                    .l()
                    .unwrap();
                if !message_obj.is_null() {
                    let message = $env
                        .get_string(message_obj.into())
                        .map(|s| String::from(s))
                        .unwrap();
                    Err(
                        $crate::jni_bridge::datafusion::error::DataFusionError::External(
                            format!(
                                "Java exception thrown at {}:{}: {}",
                                file!(),
                                line!(),
                                message
                            )
                            .into(),
                        ),
                    )
                } else {
                    Err(
                        $crate::jni_bridge::datafusion::error::DataFusionError::External(
                            format!(
                                "Java exception thrown at {}:{}: (no message)",
                                file!(),
                                line!()
                            )
                            .into(),
                        ),
                    )
                }
            }
            Err(err) => Err(
                $crate::jni_bridge::datafusion::error::DataFusionError::External(
                    format!(
                        "Unknown JNI error occurred at {}:{}: {:?}",
                        file!(),
                        line!(),
                        err
                    )
                    .into(),
                ),
            ),
        }
    }};
}

#[macro_export]
macro_rules! jni_map_error {
    ($result:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, $result))
    }};
}

#[macro_export]
macro_rules! jni_new_direct_byte_buffer {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| unsafe {
            $crate::jni_map_error_with_env!(
                env,
                env.new_direct_byte_buffer(
                    unsafe { $value.get_unchecked_mut(0) as *mut u8 },
                    $value.len()
                )
            )
        })
    }};
}

#[macro_export]
macro_rules! jni_new_string {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.new_string($value)))
    }};
}

#[macro_export]
macro_rules! jni_new_object {
    ($clsname:ident $(,$args:expr)*) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            log::trace!(
                "jni_new_object!({}, {:?})",
                stringify!($clsname),
                $crate::jvalues!($($args,)*));
            $crate::jni_map_error_with_env!(
                env,
                env.new_object_unchecked(
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].class},
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].ctor},
                    $crate::jvalues!($($args,)*))
            )
        })
    }}
}

#[macro_export]
macro_rules! jni_get_string {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            $crate::jni_map_error_with_env!(env, env.get_string($value))
                .map(|s| String::from(s))
        })
    }};
}

#[macro_export]
macro_rules! jni_get_object_class {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            $crate::jni_map_error_with_env!(env, env.get_object_class($value))
        })
    }};
}

#[macro_export]
macro_rules! jni_call {
    ($clsname:ident($obj:expr).$method:ident($($args:expr),* $(,)?) -> $ret:ty) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            log::trace!("jni_call!: {}({:?}).{}({:?})",
                stringify!($clsname),
                $obj,
                stringify!($method),
                $crate::jvalues!($($args,)*));
            $crate::jni_map_error_with_env!(
                env,
                env.call_method_unchecked(
                    $obj,
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].[<method_ $method>]},
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].[<method_ $method _ret>]}.clone(),
                    $crate::jvalues_sys!($($args,)*)
                )
            ).and_then(|result| $crate::jni_map_error_with_env!(env, <$ret>::try_from(result)))
        })
    }}
}

#[macro_export]
macro_rules! jni_call_static {
    ($clsname:ident.$method:ident($($args:expr),* $(,)?) -> $ret:ty) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            log::trace!("jni_call_static!: {}.{}({:?})",
                stringify!($clsname),
                stringify!($method),
                $crate::jvalues!($($args,)*));
            $crate::jni_map_error_with_env!(
                env,
                env.call_static_method_unchecked(
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].class},
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].[<method_ $method>]},
                    $crate::jni_bridge::paste! {$crate::jni_bridge::JavaClasses::get().[<c $clsname>].[<method_ $method _ret>]}.clone(),
                    $crate::jvalues_sys!($($args,)*)
                )
            ).and_then(|result| $crate::jni_map_error_with_env!(env, <$ret>::try_from(result)))
        })
    }}
}

#[macro_export]
macro_rules! jni_convert_byte_array {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            $crate::jni_map_error_with_env!(env, env.convert_byte_array($value))
        })
    }};
}

#[macro_export]
macro_rules! jni_new_global_ref {
    ($obj:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.new_global_ref($obj)))
    }};
}

#[macro_export]
macro_rules! jni_new_local_ref {
    ($obj:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.new_local_ref($value)))
    }};
}

#[macro_export]
macro_rules! jni_delete_local_ref {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| {
            $crate::jni_map_error_with_env!(env, env.delete_local_ref($value))
        })
    }};
}

#[macro_export]
macro_rules! jni_exception_check {
    () => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.exception_check()))
    }};
}

#[macro_export]
macro_rules! jni_exception_occurred {
    () => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.exception_occurred()))
    }};
}

#[macro_export]
macro_rules! jni_exception_describe {
    () => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.exception_describe()))
    }};
}

#[macro_export]
macro_rules! jni_exception_clear {
    () => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.exception_clear()))
    }};
}

#[macro_export]
macro_rules! jni_throw {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV
            .with(|env| $crate::jni_map_error_with_env!(env, env.throw($value)))
    }};
}

#[macro_export]
macro_rules! jni_fatal_error {
    ($value:expr) => {{
        $crate::jni_bridge::THREAD_JNIENV.with(|env| env.fatal_error($value))
    }};
}

#[allow(non_snake_case)]
pub struct JavaClasses<'a> {
    pub jvm: JavaVM,
    pub classloader: JObject<'a>,

    pub cJniBridge: JniBridge<'a>,
    pub cJniUtil: JniUtil<'a>,
    pub cClass: JavaClass<'a>,
    pub cJavaThrowable: JavaThrowable<'a>,
    pub cJavaRuntimeException: JavaRuntimeException<'a>,
    pub cJavaChannels: JavaChannels<'a>,
    pub cJavaReadableByteChannel: JavaReadableByteChannel<'a>,
    pub cJavaBoolean: JavaBoolean<'a>,
    pub cJavaLong: JavaLong<'a>,
    pub cJavaList: JavaList<'a>,
    pub cJavaMap: JavaMap<'a>,
    pub cJavaFile: JavaFile<'a>,
    pub cJavaBuffer: JavaBuffer<'a>,

    pub cScalaIterator: ScalaIterator<'a>,
    pub cScalaTuple2: ScalaTuple2<'a>,
    pub cScalaFunction0: ScalaFunction0<'a>,
    pub cScalaFunction1: ScalaFunction1<'a>,
    pub cScalaFunction2: ScalaFunction2<'a>,

    pub cHadoopFileSystem: HadoopFileSystem<'a>,
    pub cHadoopPath: HadoopPath<'a>,
    pub cHadoopFSDataInputStream: HadoopFSDataInputStream<'a>,

    pub cSparkFileSegment: SparkFileSegment<'a>,
    pub cSparkSQLMetric: SparkSQLMetric<'a>,
    pub cSparkMetricNode: SparkMetricNode<'a>,
    pub cSparkExpressionWrapperContext: SparkExpressionWrapperContext<'a>,
    pub cSparkRssShuffleWriter: SparkRssShuffleWriter<'a>,
    pub cBlazeCallNativeWrapper: BlazeCallNativeWrapper<'a>,
    pub cBlazeOnHeapSpillManager: BlazeOnHeapSpillManager<'a>,
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<'a> Send for JavaClasses<'a> {}
unsafe impl<'a> Sync for JavaClasses<'a> {}

static JNI_JAVA_CLASSES: OnceCell<JavaClasses> = OnceCell::new();

impl JavaClasses<'static> {
    pub fn init(env: &JNIEnv) {
        JNI_JAVA_CLASSES.get_or_init(|| {
            log::info!("Initializing JavaClasses...");
            let env = unsafe { std::mem::transmute::<_, &'static JNIEnv>(env) };
            let jni_bridge = JniBridge::new(env).unwrap();
            let classloader = env
                .call_static_method_unchecked(
                    jni_bridge.class,
                    jni_bridge.method_getContextClassLoader,
                    jni_bridge.method_getContextClassLoader_ret.clone(),
                    &[],
                )
                .unwrap()
                .l()
                .unwrap();

            let java_classes = JavaClasses {
                jvm: env.get_java_vm().unwrap(),
                classloader: get_global_ref_jobject(env, classloader).unwrap(),
                cJniBridge: jni_bridge,
                cJniUtil: JniUtil::new(env).unwrap(),

                cClass: JavaClass::new(env).unwrap(),
                cJavaThrowable: JavaThrowable::new(env).unwrap(),
                cJavaRuntimeException: JavaRuntimeException::new(env).unwrap(),
                cJavaChannels: JavaChannels::new(env).unwrap(),
                cJavaReadableByteChannel: JavaReadableByteChannel::new(env).unwrap(),
                cJavaBoolean: JavaBoolean::new(env).unwrap(),
                cJavaLong: JavaLong::new(env).unwrap(),
                cJavaList: JavaList::new(env).unwrap(),
                cJavaMap: JavaMap::new(env).unwrap(),
                cJavaFile: JavaFile::new(env).unwrap(),
                cJavaBuffer: JavaBuffer::new(env).unwrap(),

                cScalaIterator: ScalaIterator::new(env).unwrap(),
                cScalaTuple2: ScalaTuple2::new(env).unwrap(),
                cScalaFunction0: ScalaFunction0::new(env).unwrap(),
                cScalaFunction1: ScalaFunction1::new(env).unwrap(),
                cScalaFunction2: ScalaFunction2::new(env).unwrap(),

                cHadoopFileSystem: HadoopFileSystem::new(env).unwrap(),
                cHadoopPath: HadoopPath::new(env).unwrap(),
                cHadoopFSDataInputStream: HadoopFSDataInputStream::new(env).unwrap(),

                cSparkFileSegment: SparkFileSegment::new(env).unwrap(),
                cSparkSQLMetric: SparkSQLMetric::new(env).unwrap(),
                cSparkMetricNode: SparkMetricNode::new(env).unwrap(),
                cSparkExpressionWrapperContext: SparkExpressionWrapperContext::new(env)
                    .unwrap(),
                cSparkRssShuffleWriter: SparkRssShuffleWriter::new(env).unwrap(),
                cBlazeCallNativeWrapper: BlazeCallNativeWrapper::new(env).unwrap(),
                cBlazeOnHeapSpillManager: BlazeOnHeapSpillManager::new(env).unwrap(),
            };
            log::info!("Initializing JavaClasses finished");
            java_classes
        });
    }

    pub fn get() -> &'static JavaClasses<'static> {
        unsafe {
            // safety: JNI_JAVA_CLASSES must be initialized frist
            JNI_JAVA_CLASSES.get_unchecked()
        }
    }
}

#[allow(non_snake_case)]
pub struct JniBridge<'a> {
    pub class: JClass<'a>,
    pub method_getContextClassLoader: JStaticMethodID,
    pub method_getContextClassLoader_ret: ReturnType,
    pub method_setContextClassLoader: JStaticMethodID,
    pub method_setContextClassLoader_ret: ReturnType,
    pub method_getResource: JStaticMethodID,
    pub method_getResource_ret: ReturnType,
    pub method_setTaskContext: JStaticMethodID,
    pub method_setTaskContext_ret: ReturnType,
    pub method_getTaskContext: JStaticMethodID,
    pub method_getTaskContext_ret: ReturnType,
    pub method_getTaskOnHeapSpillManager: JStaticMethodID,
    pub method_getTaskOnHeapSpillManager_ret: ReturnType,
    pub method_isTaskRunning: JStaticMethodID,
    pub method_isTaskRunning_ret: ReturnType,
}
impl<'a> JniBridge<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/spark/sql/blaze/JniBridge";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JniBridge<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JniBridge {
            class,
            method_getContextClassLoader: env.get_static_method_id(
                class,
                "getContextClassLoader",
                "()Ljava/lang/ClassLoader;",
            )?,
            method_getContextClassLoader_ret: ReturnType::Object,
            method_setContextClassLoader: env.get_static_method_id(
                class,
                "setContextClassLoader",
                "(Ljava/lang/ClassLoader;)V",
            )?,
            method_setContextClassLoader_ret: ReturnType::Primitive(Primitive::Void),
            method_getResource: env.get_static_method_id(
                class,
                "getResource",
                "(Ljava/lang/String;)Ljava/lang/Object;",
            )?,
            method_getResource_ret: ReturnType::Object,
            method_getTaskContext: env.get_static_method_id(
                class,
                "getTaskContext",
                "()Lorg/apache/spark/TaskContext;",
            )?,
            method_getTaskContext_ret: ReturnType::Object,
            method_setTaskContext: env.get_static_method_id(
                class,
                "setTaskContext",
                "(Lorg/apache/spark/TaskContext;)V",
            )?,
            method_setTaskContext_ret: ReturnType::Primitive(Primitive::Void),
            method_getTaskOnHeapSpillManager: env.get_static_method_id(
                class,
                "getTaskOnHeapSpillManager",
                "()Lorg/apache/spark/sql/blaze/memory/OnHeapSpillManager;",
            )?,
            method_getTaskOnHeapSpillManager_ret: ReturnType::Object,
            method_isTaskRunning: env.get_static_method_id(
                class,
                "isTaskRunning",
                "()Z",
            )?,
            method_isTaskRunning_ret: ReturnType::Primitive(Primitive::Boolean),
        })
    }
}

#[allow(non_snake_case)]
pub struct JniUtil<'a> {
    pub class: JClass<'a>,
    pub method_readFullyFromFSDataInputStream: JStaticMethodID,
    pub method_readFullyFromFSDataInputStream_ret: ReturnType,
}
impl<'a> JniUtil<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/spark/sql/blaze/JniUtil";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JniUtil<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JniUtil {
            class,
            method_readFullyFromFSDataInputStream: env.get_static_method_id(
                class,
                "readFullyFromFSDataInputStream",
                "(Lorg/apache/hadoop/fs/FSDataInputStream;JLjava/nio/ByteBuffer;)V",
            )?,
            method_readFullyFromFSDataInputStream_ret: ReturnType::Primitive(
                Primitive::Void,
            ),
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaClass<'a> {
    pub class: JClass<'a>,
    pub method_getName: JMethodID,
    pub method_getName_ret: ReturnType,
}
impl<'a> JavaClass<'a> {
    pub const SIG_TYPE: &'static str = "java/lang/Class";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaClass<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaClass {
            class,
            method_getName: env.get_method_id(
                class,
                "getName",
                "()Ljava/lang/String;",
            )?,
            method_getName_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaThrowable<'a> {
    pub class: JClass<'a>,
    pub method_getMessage: JMethodID,
    pub method_getMessage_ret: ReturnType,
}
impl<'a> JavaThrowable<'a> {
    pub const SIG_TYPE: &'static str = "java/lang/Throwable";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaThrowable<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaThrowable {
            class,
            method_getMessage: env.get_method_id(
                class,
                "getMessage",
                "()Ljava/lang/String;",
            )?,
            method_getMessage_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaRuntimeException<'a> {
    pub class: JClass<'a>,
    pub ctor: JMethodID,
}
impl<'a> JavaRuntimeException<'a> {
    pub const SIG_TYPE: &'static str = "java/lang/RuntimeException";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaRuntimeException<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaRuntimeException {
            class,
            ctor: env.get_method_id(
                class,
                "<init>",
                "(Ljava/lang/String;Ljava/lang/Throwable;)V",
            )?,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaChannels<'a> {
    pub class: JClass<'a>,
    pub method_newChannel: JStaticMethodID,
    pub method_newChannel_ret: ReturnType,
}
impl<'a> JavaChannels<'a> {
    pub const SIG_TYPE: &'static str = "java/nio/channels/Channels";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaChannels<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaChannels {
            class,
            method_newChannel: env.get_static_method_id(
                class,
                "newChannel",
                "(Ljava/io/InputStream;)Ljava/nio/channels/ReadableByteChannel;",
            )?,
            method_newChannel_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaReadableByteChannel<'a> {
    pub class: JClass<'a>,
    pub method_read: JMethodID,
    pub method_read_ret: ReturnType,
    pub method_close: JMethodID,
    pub method_close_ret: ReturnType,
}
impl<'a> JavaReadableByteChannel<'a> {
    pub const SIG_TYPE: &'static str = "java/nio/channels/ReadableByteChannel";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaReadableByteChannel<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaReadableByteChannel {
            class,
            method_read: env.get_method_id(class, "read", "(Ljava/nio/ByteBuffer;)I")?,
            method_read_ret: ReturnType::Primitive(Primitive::Int),
            method_close: env.get_method_id(class, "close", "()V")?,
            method_close_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaBoolean<'a> {
    pub class: JClass<'a>,
    pub ctor: JMethodID,
}
impl<'a> JavaBoolean<'a> {
    pub const SIG_TYPE: &'static str = "java/lang/Boolean";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaBoolean<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaBoolean {
            class,
            ctor: env.get_method_id(class, "<init>", "(Z)V")?,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaLong<'a> {
    pub class: JClass<'a>,
    pub ctor: JMethodID,
    pub method_longValue: JMethodID,
    pub method_longValue_ret: ReturnType,
}
impl<'a> JavaLong<'a> {
    pub const SIG_TYPE: &'static str = "java/lang/Long";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaLong<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaLong {
            class,
            ctor: env.get_method_id(class, "<init>", "(J)V")?,
            method_longValue: env.get_method_id(class, "longValue", "()J")?,
            method_longValue_ret: ReturnType::Primitive(Primitive::Long),
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaList<'a> {
    pub class: JClass<'a>,
    pub method_size: JMethodID,
    pub method_size_ret: ReturnType,
    pub method_get: JMethodID,
    pub method_get_ret: ReturnType,
}
impl<'a> JavaList<'a> {
    pub const SIG_TYPE: &'static str = "java/util/List";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaList<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaList {
            class,
            method_size: env.get_method_id(class, "size", "()I")?,
            method_size_ret: ReturnType::Primitive(Primitive::Int),
            method_get: env.get_method_id(class, "get", "(I)Ljava/lang/Object;")?,
            method_get_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaMap<'a> {
    pub class: JClass<'a>,
    pub method_get: JMethodID,
    pub method_get_ret: ReturnType,
    pub method_put: JMethodID,
    pub method_put_ret: ReturnType,
}
impl<'a> JavaMap<'a> {
    pub const SIG_TYPE: &'static str = "java/util/Map";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaMap<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaMap {
            class,
            method_get: env
                .get_method_id(class, "get", "(Ljava/lang/Object;)Ljava/lang/Object;")
                .unwrap(),
            method_get_ret: ReturnType::Object,
            method_put: env
                .get_method_id(
                    class,
                    "put",
                    "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                )
                .unwrap(),
            method_put_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaFile<'a> {
    pub class: JClass<'a>,
    pub method_getPath: JMethodID,
    pub method_getPath_ret: ReturnType,
}
impl<'a> JavaFile<'a> {
    pub const SIG_TYPE: &'static str = "java/io/File";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaFile<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaFile {
            class,
            method_getPath: env.get_method_id(
                class,
                "getPath",
                "()Ljava/lang/String;",
            )?,
            method_getPath_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct JavaBuffer<'a> {
    pub class: JClass<'a>,
    pub method_hasRemaining: JMethodID,
    pub method_hasRemaining_ret: ReturnType,
    pub method_position: JMethodID,
    pub method_position_ret: ReturnType,
}
impl<'a> JavaBuffer<'a> {
    pub const SIG_TYPE: &'static str = "java/nio/Buffer";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<JavaBuffer<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(JavaBuffer {
            class,
            method_hasRemaining: env.get_method_id(class, "hasRemaining", "()Z")?,
            method_hasRemaining_ret: ReturnType::Primitive(Primitive::Boolean),
            method_position: env.get_method_id(class, "position", "()I")?,
            method_position_ret: ReturnType::Primitive(Primitive::Int),
        })
    }
}

#[allow(non_snake_case)]
pub struct ScalaIterator<'a> {
    pub class: JClass<'a>,
    pub method_hasNext: JMethodID,
    pub method_hasNext_ret: ReturnType,
    pub method_next: JMethodID,
    pub method_next_ret: ReturnType,
}
impl<'a> ScalaIterator<'a> {
    pub const SIG_TYPE: &'static str = "scala/collection/Iterator";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<ScalaIterator<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(ScalaIterator {
            class,
            method_hasNext: env.get_method_id(class, "hasNext", "()Z")?,
            method_hasNext_ret: ReturnType::Primitive(Primitive::Boolean),
            method_next: env.get_method_id(class, "next", "()Ljava/lang/Object;")?,
            method_next_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct ScalaTuple2<'a> {
    pub class: JClass<'a>,
    pub method__1: JMethodID,
    pub method__1_ret: ReturnType,
    pub method__2: JMethodID,
    pub method__2_ret: ReturnType,
}
impl<'a> ScalaTuple2<'a> {
    pub const SIG_TYPE: &'static str = "scala/Tuple2";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<ScalaTuple2<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(ScalaTuple2 {
            class,
            method__1: env.get_method_id(class, "_1", "()Ljava/lang/Object;")?,
            method__1_ret: ReturnType::Object,
            method__2: env.get_method_id(class, "_2", "()Ljava/lang/Object;")?,
            method__2_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct ScalaFunction0<'a> {
    pub class: JClass<'a>,
    pub method_apply: JMethodID,
    pub method_apply_ret: ReturnType,
}
impl<'a> ScalaFunction0<'a> {
    pub const SIG_TYPE: &'static str = "scala/Function0";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<ScalaFunction0<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(ScalaFunction0 {
            class,
            method_apply: env.get_method_id(class, "apply", "()Ljava/lang/Object;")?,
            method_apply_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct ScalaFunction1<'a> {
    pub class: JClass<'a>,
    pub method_apply: JMethodID,
    pub method_apply_ret: ReturnType,
}
impl<'a> ScalaFunction1<'a> {
    pub const SIG_TYPE: &'static str = "scala/Function1";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<ScalaFunction1<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(ScalaFunction1 {
            class,
            method_apply: env.get_method_id(
                class,
                "apply",
                "(Ljava/lang/Object;)Ljava/lang/Object;",
            )?,
            method_apply_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct ScalaFunction2<'a> {
    pub class: JClass<'a>,
    pub method_apply: JMethodID,
    pub method_apply_ret: ReturnType,
}
impl<'a> ScalaFunction2<'a> {
    pub const SIG_TYPE: &'static str = "scala/Function2";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<ScalaFunction2<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(ScalaFunction2 {
            class,
            method_apply: env.get_method_id(
                class,
                "apply",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            )?,
            method_apply_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct HadoopFileSystem<'a> {
    pub class: JClass<'a>,
    pub method_open: JMethodID,
    pub method_open_ret: ReturnType,
}
impl<'a> HadoopFileSystem<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/hadoop/fs/FileSystem";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<HadoopFileSystem<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(HadoopFileSystem {
            class,
            method_open: env.get_method_id(
                class,
                "open",
                "(Lorg/apache/hadoop/fs/Path;)Lorg/apache/hadoop/fs/FSDataInputStream;",
            )?,
            method_open_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct HadoopPath<'a> {
    pub class: JClass<'a>,
    pub ctor: JMethodID,
}
impl<'a> HadoopPath<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/hadoop/fs/Path";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<HadoopPath<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(HadoopPath {
            class,
            ctor: env.get_method_id(class, "<init>", "(Ljava/lang/String;)V")?,
        })
    }
}

#[allow(non_snake_case)]
pub struct HadoopFSDataInputStream<'a> {
    pub class: JClass<'a>,
    pub method_seek: JMethodID,
    pub method_seek_ret: ReturnType,
    pub method_close: JMethodID,
    pub method_close_ret: ReturnType,
}
impl<'a> HadoopFSDataInputStream<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/hadoop/fs/FSDataInputStream";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<HadoopFSDataInputStream<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(HadoopFSDataInputStream {
            class,
            method_seek: env.get_method_id(class, "seek", "(J)V")?,
            method_seek_ret: ReturnType::Primitive(Primitive::Void),
            method_close: env.get_method_id(class, "close", "()V")?,
            method_close_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct SparkFileSegment<'a> {
    pub class: JClass<'a>,
    pub method_file: JMethodID,
    pub method_file_ret: ReturnType,
    pub method_offset: JMethodID,
    pub method_offset_ret: ReturnType,
    pub method_length: JMethodID,
    pub method_length_ret: ReturnType,
}
impl<'a> SparkFileSegment<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/spark/storage/FileSegment";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<SparkFileSegment<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(SparkFileSegment {
            class,
            method_file: env.get_method_id(class, "file", "()Ljava/io/File;")?,
            method_file_ret: ReturnType::Object,
            method_offset: env.get_method_id(class, "offset", "()J")?,
            method_offset_ret: ReturnType::Primitive(Primitive::Long),
            method_length: env.get_method_id(class, "length", "()J")?,
            method_length_ret: ReturnType::Primitive(Primitive::Long),
        })
    }
}

#[allow(non_snake_case)]
pub struct SparkSQLMetric<'a> {
    pub class: JClass<'a>,
    pub method_add: JMethodID,
    pub method_add_ret: ReturnType,
}
impl<'a> SparkSQLMetric<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/spark/sql/execution/metric/SQLMetric";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<SparkSQLMetric<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(SparkSQLMetric {
            class,
            method_add: env.get_method_id(class, "add", "(J)V")?,
            method_add_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct SparkMetricNode<'a> {
    pub class: JClass<'a>,
    pub method_getChild: JMethodID,
    pub method_getChild_ret: ReturnType,
    pub method_add: JMethodID,
    pub method_add_ret: ReturnType,
}
impl<'a> SparkMetricNode<'a> {
    pub const SIG_TYPE: &'static str = "org/apache/spark/sql/blaze/MetricNode";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<SparkMetricNode<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(SparkMetricNode {
            class,
            method_getChild: env
                .get_method_id(
                    class,
                    "getChild",
                    "(I)Lorg/apache/spark/sql/blaze/MetricNode;",
                )
                .unwrap(),
            method_getChild_ret: ReturnType::Object,
            method_add: env
                .get_method_id(class, "add", "(Ljava/lang/String;J)V")
                .unwrap(),
            method_add_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct SparkRssShuffleWriter<'a> {
    pub class: JClass<'a>,
    pub method_write: JMethodID,
    pub method_write_ret: ReturnType,
    pub method_close: JMethodID,
    pub method_close_ret: ReturnType,
}

impl<'a> SparkRssShuffleWriter<'_> {
    pub const SIG_TYPE: &'static str =
        "org/apache/spark/sql/execution/blaze/shuffle/RssPartitionWriterBase";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<SparkRssShuffleWriter<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(SparkRssShuffleWriter {
            class,
            method_write: env
                .get_method_id(class, "write", "(ILjava/nio/ByteBuffer;I)V")
                .unwrap(),
            method_write_ret: ReturnType::Primitive(Primitive::Void),
            method_close: env.get_method_id(class, "close", "(I)V").unwrap(),
            method_close_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct SparkExpressionWrapperContext<'a> {
    pub class: JClass<'a>,
    pub ctor: JMethodID,
    pub method_eval: JMethodID,
    pub method_eval_ret: ReturnType,
}
impl<'a> SparkExpressionWrapperContext<'a> {
    pub const SIG_TYPE: &'static str =
        "org/apache/spark/sql/blaze/SparkExpressionWrapperContext";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<SparkExpressionWrapperContext<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(SparkExpressionWrapperContext {
            class,
            ctor: env.get_method_id(class, "<init>", "(Ljava/nio/ByteBuffer;)V")?,
            method_eval: env
                .get_method_id(
                    class,
                    "eval",
                    "(Ljava/nio/ByteBuffer;)Ljava/nio/channels/ReadableByteChannel;",
                )
                .unwrap(),
            method_eval_ret: ReturnType::Object,
        })
    }
}

#[allow(non_snake_case)]
pub struct BlazeCallNativeWrapper<'a> {
    pub class: JClass<'a>,
    pub method_isFinished: JMethodID,
    pub method_isFinished_ret: ReturnType,
    pub method_getRawTaskDefinition: JMethodID,
    pub method_getRawTaskDefinition_ret: ReturnType,
    pub method_getMetrics: JMethodID,
    pub method_getMetrics_ret: ReturnType,
    pub method_enqueueWithTimeout: JMethodID,
    pub method_enqueueWithTimeout_ret: ReturnType,
    pub method_enqueueError: JMethodID,
    pub method_enqueueError_ret: ReturnType,
    pub method_dequeueWithTimeout: JMethodID,
    pub method_dequeueWithTimeout_ret: ReturnType,
    pub method_finishNativeThread: JMethodID,
    pub method_finishNativeThread_ret: ReturnType,
}
impl<'a> BlazeCallNativeWrapper<'a> {
    pub const SIG_TYPE: &'static str =
        "org/apache/spark/sql/blaze/BlazeCallNativeWrapper";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<BlazeCallNativeWrapper<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(BlazeCallNativeWrapper {
            class,
            method_isFinished: env.get_method_id(class, "isFinished", "()Z").unwrap(),
            method_isFinished_ret: ReturnType::Primitive(Primitive::Boolean),
            method_getRawTaskDefinition: env
                .get_method_id(class, "getRawTaskDefinition", "()[B")
                .unwrap(),
            method_getRawTaskDefinition_ret: ReturnType::Array,
            method_getMetrics: env
                .get_method_id(
                    class,
                    "getMetrics",
                    "()Lorg/apache/spark/sql/blaze/MetricNode;",
                )
                .unwrap(),
            method_getMetrics_ret: ReturnType::Object,
            method_enqueueWithTimeout: env
                .get_method_id(class, "enqueueWithTimeout", "(Ljava/lang/Object;)Z")
                .unwrap(),
            method_enqueueWithTimeout_ret: ReturnType::Primitive(Primitive::Boolean),
            method_enqueueError: env
                .get_method_id(class, "enqueueError", "(Ljava/lang/Object;)Z")
                .unwrap(),
            method_enqueueError_ret: ReturnType::Primitive(Primitive::Boolean),
            method_dequeueWithTimeout: env
                .get_method_id(class, "dequeueWithTimeout", "()Ljava/lang/Object;")
                .unwrap(),
            method_dequeueWithTimeout_ret: ReturnType::Object,
            method_finishNativeThread: env
                .get_method_id(class, "finishNativeThread", "()V")
                .unwrap(),
            method_finishNativeThread_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

#[allow(non_snake_case)]
pub struct BlazeOnHeapSpillManager<'a> {
    pub class: JClass<'a>,
    pub method_newSpill: JMethodID,
    pub method_newSpill_ret: ReturnType,
    pub method_writeSpill: JMethodID,
    pub method_writeSpill_ret: ReturnType,
    pub method_completeSpill: JMethodID,
    pub method_completeSpill_ret: ReturnType,
    pub method_readSpill: JMethodID,
    pub method_readSpill_ret: ReturnType,
    pub method_releaseSpill: JMethodID,
    pub method_releaseSpill_ret: ReturnType,
}
impl<'a> BlazeOnHeapSpillManager<'a> {
    pub const SIG_TYPE: &'static str =
        "org/apache/spark/sql/blaze/memory/OnHeapSpillManager";

    pub fn new(env: &JNIEnv<'a>) -> JniResult<BlazeOnHeapSpillManager<'a>> {
        let class = get_global_jclass(env, Self::SIG_TYPE)?;
        Ok(BlazeOnHeapSpillManager {
            class,
            method_newSpill: env.get_method_id(class, "newSpill", "(J)I").unwrap(),
            method_newSpill_ret: ReturnType::Primitive(Primitive::Int),
            method_writeSpill: env
                .get_method_id(class, "writeSpill", "(ILjava/nio/ByteBuffer;)V")
                .unwrap(),
            method_writeSpill_ret: ReturnType::Primitive(Primitive::Void),
            method_completeSpill: env
                .get_method_id(class, "completeSpill", "(I)V")
                .unwrap(),
            method_completeSpill_ret: ReturnType::Primitive(Primitive::Void),
            method_readSpill: env
                .get_method_id(class, "readSpill", "(ILjava/nio/ByteBuffer;)I")
                .unwrap(),
            method_readSpill_ret: ReturnType::Primitive(Primitive::Int),
            method_releaseSpill: env
                .get_method_id(class, "releaseSpill", "(I)V")
                .unwrap(),
            method_releaseSpill_ret: ReturnType::Primitive(Primitive::Void),
        })
    }
}

fn get_global_jclass<'a>(env: &JNIEnv<'a>, cls: &str) -> JniResult<JClass<'static>> {
    let local_jclass = env.find_class(cls)?;
    Ok(get_global_ref_jobject(env, local_jclass.into())?.into())
}

fn get_global_ref_jobject<'a>(
    env: &JNIEnv<'a>,
    obj: JObject<'a>,
) -> JniResult<JObject<'static>> {
    let global = env.new_global_ref::<JObject>(obj)?;

    // safety:
    //  as all global refs to jclass in JavaClasses should never be GC'd during
    // the whole jvm lifetime, we put GlobalRef into ManuallyDrop to prevent
    // deleting these global refs.
    let global_obj =
        unsafe { std::mem::transmute::<_, JObject<'static>>(global.as_obj()) };
    let _ = std::mem::ManuallyDrop::new(global);
    Ok(global_obj)
}
