//! One trait, three engines. Each engine owns one isolated JS heap with the bundle loaded and
//! exposes nothing to the script but the call into `__decide`.

use std::time::Duration;

pub trait Engine: Sized {
    const NAME: &'static str;
    fn load(bundle: &str) -> Self;
    /// Calls one of the bundle's global functions with string arguments; returns its string.
    fn call(&mut self, function: &str, args: &[&str]) -> Result<String, String>;
    /// The same call, stopped by the bridge when it runs past `budget`.
    fn call_with_budget(&mut self, function: &str, args: &[&str], budget: Duration) -> Result<String, String>;
    fn heap_bytes(&mut self) -> Option<usize>;

    fn decide(&mut self, program: &str, history: &str) -> Result<String, String> {
        self.call("__decide", &[program, history])
    }
    fn decide_with_budget(&mut self, program: &str, history: &str, budget: Duration) -> Result<String, String> {
        self.call_with_budget("__decide", &[program, history], budget)
    }
}

#[cfg(feature = "quickjs")]
pub mod quickjs {
    use super::*;
    use rquickjs::{Context, Function, Runtime};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    pub struct QuickJs {
        runtime: Runtime,
        context: Context,
        // Nanoseconds after `epoch` at which the interrupt handler stops the script; 0 is no limit.
        deadline: Arc<AtomicU64>,
        epoch: Instant,
    }

    impl Engine for QuickJs {
        const NAME: &'static str = "quickjs (rquickjs 0.14)";

        fn load(bundle: &str) -> Self {
            let runtime = Runtime::new().unwrap();
            runtime.set_memory_limit(256 << 20);
            runtime.set_max_stack_size(4 << 20);
            let deadline = Arc::new(AtomicU64::new(0));
            let epoch = Instant::now();
            let d = deadline.clone();
            runtime.set_interrupt_handler(Some(Box::new(move || {
                let limit = d.load(Ordering::Relaxed);
                limit != 0 && epoch.elapsed().as_nanos() as u64 > limit
            })));
            // Context::full gives ECMAScript intrinsics only: QuickJS has no host I/O unless the
            // embedder adds it, and this bridge adds none.
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| ctx.eval::<(), _>(bundle).map_err(|e| format!("{e:?}")).unwrap());
            QuickJs { runtime, context, deadline, epoch }
        }

        fn call(&mut self, function: &str, args: &[&str]) -> Result<String, String> {
            self.context.with(|ctx| {
                let f: Function = ctx.globals().get(function).map_err(|e| e.to_string())?;
                let result = match args {
                    [a] => f.call::<_, String>((*a,)),
                    [a, b] => f.call::<_, String>((*a, *b)),
                    _ => return Err("one or two arguments".to_owned()),
                };
                result.map_err(|e| {
                    let caught = ctx.catch();
                    format!("{e}: {caught:?}")
                })
            })
        }

        fn call_with_budget(&mut self, function: &str, args: &[&str], budget: Duration) -> Result<String, String> {
            let limit = self.epoch.elapsed() + budget;
            self.deadline.store(limit.as_nanos() as u64, Ordering::Relaxed);
            let out = self.call(function, args);
            self.deadline.store(0, Ordering::Relaxed);
            out
        }

        fn heap_bytes(&mut self) -> Option<usize> {
            Some(self.runtime.memory_usage().memory_used_size as usize)
        }
    }
}

#[cfg(feature = "boa")]
pub mod boa {
    use super::*;
    use boa_engine::{Context, JsString, JsValue, Source};

    pub struct Boa {
        context: Context,
    }

    impl Engine for Boa {
        const NAME: &'static str = "boa (boa_engine 0.22)";

        fn load(bundle: &str) -> Self {
            // Context::default registers ECMAScript built-ins and no host functions.
            let mut context = Context::default();
            context.runtime_limits_mut().set_recursion_limit(4096);
            context.eval(Source::from_bytes(bundle)).map_err(|e| e.to_string()).unwrap();
            Boa { context }
        }

        fn call(&mut self, function: &str, args: &[&str]) -> Result<String, String> {
            let ctx = &mut self.context;
            let f = ctx.global_object().get(JsString::from(function), ctx).map_err(|e| e.to_string())?;
            let f = f.as_callable().ok_or("not a function")?.clone();
            let args: Vec<JsValue> = args.iter().map(|a| JsValue::from(JsString::from(*a))).collect();
            let out = f.call(&JsValue::undefined(), &args, ctx).map_err(|e| e.to_string())?;
            Ok(out.as_string().ok_or("the call returned no string")?.to_std_string_escaped())
        }

        // Boa has no time-based interrupt. Its closest guard is a loop-iteration limit, which
        // stops `for (;;) {}` but not a slow loop that finishes, so this is a weaker guarantee.
        fn call_with_budget(&mut self, function: &str, args: &[&str], _budget: Duration) -> Result<String, String> {
            self.context.runtime_limits_mut().set_loop_iteration_limit(10_000_000);
            let out = self.call(function, args);
            self.context.runtime_limits_mut().set_loop_iteration_limit(u64::MAX);
            out
        }

        fn heap_bytes(&mut self) -> Option<usize> {
            None
        }
    }
}

#[cfg(feature = "v8")]
pub mod v8 {
    use super::*;
    use deno_core::{JsRuntime, RuntimeOptions, scope, v8};
    use std::sync::mpsc;

    pub struct V8 {
        runtime: JsRuntime,
        functions: std::collections::HashMap<String, v8::Global<v8::Function>>,
    }

    fn finish(mut runtime: JsRuntime, load_bundle: Option<&str>) -> V8 {
        if let Some(bundle) = load_bundle {
            runtime.execute_script("bundle.js", bundle.to_owned()).map_err(|e| e.to_string()).unwrap();
        }
        let mut functions = std::collections::HashMap::new();
        for name in ["__decide", "__open", "__apply", "__close"] {
            let value = runtime.execute_script("lookup.js", format!("globalThis.{name}")).unwrap();
            scope!(scope, runtime);
            let local = v8::Local::new(scope, value);
            let f: v8::Local<v8::Function> = local.try_into().unwrap();
            functions.insert(name.to_owned(), v8::Global::new(scope, f));
        }
        V8 { runtime, functions }
    }

    fn call(engine: &mut V8, function: &str, args: &[&str]) -> Result<String, String> {
        let global = engine.functions.get(function).ok_or("unknown function")?.clone();
        let runtime = &mut engine.runtime;
        scope!(scope, runtime);
        v8::tc_scope!(let tc, scope);
        let f = v8::Local::new(tc, &global);
        let args: Vec<v8::Local<v8::Value>> = args.iter().map(|a| v8::String::new(tc, a).unwrap().into()).collect();
        let this = v8::undefined(tc).into();
        match f.call(tc, this, &args) {
            Some(v) => Ok(v.to_rust_string_lossy(tc)),
            None => Err(match tc.exception() {
                Some(e) => e.to_rust_string_lossy(tc),
                None => "terminated".to_owned(),
            }),
        }
    }

    fn budgeted(engine: &mut V8, function: &str, args: &[&str], budget: Duration) -> Result<String, String> {
        let handle = engine.runtime.v8_isolate().thread_safe_handle();
        let (done, wait) = mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            if wait.recv_timeout(budget).is_err() {
                handle.terminate_execution();
            }
        });
        let out = call(engine, function, args);
        let _ = done.send(());
        watchdog.join().unwrap();
        engine.runtime.v8_isolate().cancel_terminate_execution();
        out
    }

    fn heap(engine: &mut V8) -> Option<usize> {
        let stats = engine.runtime.v8_isolate().get_heap_statistics();
        Some(stats.used_heap_size())
    }

    impl Engine for V8 {
        const NAME: &'static str = "v8 (deno_core 0.412, no snapshot)";
        fn load(bundle: &str) -> Self {
            finish(JsRuntime::new(RuntimeOptions::default()), Some(bundle))
        }
        fn call(&mut self, function: &str, args: &[&str]) -> Result<String, String> {
            call(self, function, args)
        }
        fn call_with_budget(&mut self, function: &str, args: &[&str], budget: Duration) -> Result<String, String> {
            budgeted(self, function, args, budget)
        }
        fn heap_bytes(&mut self) -> Option<usize> {
            heap(self)
        }
    }

    /// V8 started from a snapshot that already holds the evaluated bundle. The snapshot is made
    /// by a separate process (`mission-bridge v8-make-snapshot`), as a build step would.
    pub struct V8Snapshot(V8);

    static SNAPSHOT: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

    pub fn make_snapshot(bundle: &str, out: &std::path::Path) {
        let mut runtime = deno_core::JsRuntimeForSnapshot::new(RuntimeOptions::default());
        runtime.execute_script("bundle.js", bundle.to_owned()).unwrap();
        std::fs::write(out, runtime.snapshot()).unwrap();
    }

    impl Engine for V8Snapshot {
        const NAME: &'static str = "v8 (deno_core 0.412, startup snapshot)";
        fn load(_bundle: &str) -> Self {
            let snapshot = SNAPSHOT.get_or_init(|| {
                let path = std::env::var("V8_SNAPSHOT").unwrap_or_else(|_| "fixtures/bundle.snapshot".into());
                Box::leak(std::fs::read(path).expect("make the snapshot first").into_boxed_slice())
            });
            let runtime = JsRuntime::new(RuntimeOptions { startup_snapshot: Some(snapshot), ..Default::default() });
            V8Snapshot(finish(runtime, None))
        }
        fn call(&mut self, function: &str, args: &[&str]) -> Result<String, String> {
            call(&mut self.0, function, args)
        }
        fn call_with_budget(&mut self, function: &str, args: &[&str], budget: Duration) -> Result<String, String> {
            budgeted(&mut self.0, function, args, budget)
        }
        fn heap_bytes(&mut self) -> Option<usize> {
            heap(&mut self.0)
        }
    }
}
