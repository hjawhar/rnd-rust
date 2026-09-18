//! Rhai scripting engine for user-defined IRC automation.
//!
//! Provides a sandboxed [`ScriptEngine`] that exposes IRC operations
//! (sending messages, joining channels, etc.) to Rhai scripts while
//! enforcing resource limits to prevent runaway execution.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::RwLock;
use rhai::packages::Package;
use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use tokio::sync::mpsc;

use crate::command::ClientCommand;
use crate::event::NetworkId;

/// Errors produced by script evaluation.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// A Rhai evaluation error.
    #[error("script error: {0}")]
    Eval(#[from] Box<EvalAltResult>),

    /// Failed to read a script file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A Rhai parse error.
    #[error("parse error: {0}")]
    Parse(#[from] rhai::ParseError),
}

/// Shared state accessible from within Rhai scripts.
#[derive(Clone)]
struct ScriptContext {
    command_tx: mpsc::UnboundedSender<ClientCommand>,
    network_id: NetworkId,
    current_nick: Arc<RwLock<String>>,
    current_channel: Arc<RwLock<String>>,
}

/// The scripting engine wraps a Rhai `Engine` configured with
/// IRC-specific functions and safety limits.
pub struct ScriptEngine {
    engine: Engine,
    scope: Scope<'static>,
    /// Combined AST of all previously evaluated scripts, so that
    /// function definitions persist across `eval` calls.
    ast: rhai::AST,
    aliases: HashMap<String, String>,
    hooks: HashMap<String, String>,
    ctx: ScriptContext,
}

impl ScriptEngine {
    /// Creates a new scripting engine wired to the given command channel.
    ///
    /// The engine is sandboxed: filesystem access is disabled, and
    /// CPU/memory limits are applied to prevent runaway scripts.
    pub fn new(
        command_tx: mpsc::UnboundedSender<ClientCommand>,
        network_id: NetworkId,
        nick: String,
        channel: String,
    ) -> Self {
        let ctx = ScriptContext {
            command_tx,
            network_id,
            current_nick: Arc::new(RwLock::new(nick)),
            current_channel: Arc::new(RwLock::new(channel)),
        };

        let mut engine = Engine::new_raw();

        // Safety limits.
        engine.set_max_operations(100_000);
        engine.set_max_modules(10);
        engine.set_max_string_size(10_000);

        // Disable filesystem access.
        engine.disable_symbol("import");

        // Register the core language package so arithmetic/strings/etc. work.
        let core_pkg = rhai::packages::CorePackage::new();
        engine.register_global_module(core_pkg.as_shared_module());

        // Register standard packages for basic operations.
        let std_pkg = rhai::packages::StandardPackage::new();
        engine.register_global_module(std_pkg.as_shared_module());

        // --- IRC helper functions ---

        let c = ctx.clone();
        engine.register_fn("send_msg", move |target: &str, text: &str| {
            let _ = c.command_tx.send(ClientCommand::SendPrivmsg {
                network: c.network_id,
                target: Bytes::from(target.to_owned()),
                text: Bytes::from(text.to_owned()),
            });
        });

        let c = ctx.clone();
        engine.register_fn("send_raw", move |line: &str| {
            let _ = c.command_tx.send(ClientCommand::SendRaw {
                network: c.network_id,
                line: Bytes::from(line.to_owned()),
            });
        });

        let c = ctx.clone();
        engine.register_fn("join", move |channel: &str| {
            let _ = c.command_tx.send(ClientCommand::Join {
                network: c.network_id,
                channel: Bytes::from(channel.to_owned()),
            });
        });

        let c = ctx.clone();
        engine.register_fn("part", move |channel: &str| {
            let _ = c.command_tx.send(ClientCommand::Part {
                network: c.network_id,
                channel: Bytes::from(channel.to_owned()),
                reason: None,
            });
        });

        // `echo` prints locally — in a real client this would push to a
        // scrollback buffer, but for now we use tracing.
        engine.register_fn("echo", |text: &str| {
            tracing::info!(script_echo = %text);
        });

        let c = ctx.clone();
        engine.register_fn("nick", move || -> String { c.current_nick.read().clone() });

        let c = ctx.clone();
        engine.register_fn("channel", move || -> String {
            c.current_channel.read().clone()
        });

        engine.register_fn("version", || -> String {
            env!("CARGO_PKG_VERSION").to_owned()
        });

        Self {
            ast: rhai::AST::empty(),
            engine,
            scope: Scope::new(),
            aliases: HashMap::new(),
            hooks: HashMap::new(),
            ctx,
        }
    }

    /// Update the current nick visible to scripts.
    pub fn set_nick(&self, nick: &str) {
        nick.clone_into(&mut self.ctx.current_nick.write());
    }

    /// Update the current channel visible to scripts.
    pub fn set_channel(&self, channel: &str) {
        channel.clone_into(&mut self.ctx.current_channel.write());
    }

    /// Evaluate a Rhai script string.
    pub fn eval(&mut self, script: &str) -> Result<(), ScriptError> {
        let new_ast = self.engine.compile(script)?;
        // Combine: new statements run, but all previously defined
        // functions remain available.
        let combined = self.ast.merge(&new_ast);
        self.engine.run_ast_with_scope(&mut self.scope, &combined)?;
        // Retain only function definitions for future calls.
        let mut fns_only = combined;
        fns_only.clear_statements();
        self.ast = fns_only;
        Ok(())
    }

    /// Evaluate a Rhai script from a file path.
    pub fn eval_file(&mut self, path: &Path) -> Result<(), ScriptError> {
        let source = std::fs::read_to_string(path)?;
        self.eval(&source)
    }

    /// Register a command alias.
    ///
    /// When the user types `/name args`, the engine evaluates `body`
    /// with the variable `args` bound to the argument string.
    pub fn register_alias(&mut self, name: &str, body: &str) -> Result<(), ScriptError> {
        // Compile-check the body before accepting it.
        self.engine.compile(body)?;
        self.aliases.insert(name.to_owned(), body.to_owned());
        Ok(())
    }

    /// Invoke a previously registered alias with the given arguments.
    pub fn run_alias(&mut self, name: &str, args: &str) -> Result<(), ScriptError> {
        let body = self
            .aliases
            .get(name)
            .ok_or_else(|| {
                Box::new(EvalAltResult::ErrorSystem(
                    "alias not found".into(),
                    format!("no alias named '{name}'").into(),
                ))
            })?
            .clone();

        self.scope.push("args", args.to_owned());
        let body_ast = self.engine.compile(&body)?;
        let combined = self.ast.merge(&body_ast);
        let result = self.engine.run_ast_with_scope(&mut self.scope, &combined);
        let _ = self.scope.remove::<String>("args");
        result?;
        Ok(())
    }

    /// Register an event hook: when `event` fires, the Rhai function
    /// `handler_fn` (which must already be defined in the scope) is called.
    pub fn register_hook(&mut self, event: &str, handler_fn: &str) -> Result<(), ScriptError> {
        self.hooks.insert(event.to_owned(), handler_fn.to_owned());
        Ok(())
    }

    /// Fire an event, invoking any registered hook function.
    pub fn fire_event(&mut self, event: &str, args: &[Dynamic]) -> Result<(), ScriptError> {
        if let Some(handler) = self.hooks.get(event).cloned() {
            // Build a call expression: handler(arg0, arg1, ...)
            let arg_names: Vec<String> = (0..args.len()).map(|i| format!("__arg{i}")).collect();
            for (i, arg) in args.iter().enumerate() {
                self.scope.push(arg_names[i].clone(), arg.clone());
            }

            let call_args = arg_names.join(", ");
            let call_expr = format!("{handler}({call_args})");
            let call_ast = self.engine.compile(&call_expr)?;
            let combined = self.ast.merge(&call_ast);
            let result = self.engine.run_ast_with_scope(&mut self.scope, &combined);

            // Clean up temporary variables.
            for name in &arg_names {
                let _ = self.scope.remove::<Dynamic>(name);
            }

            result?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a `ScriptEngine` with a dummy command channel.
    fn test_engine() -> (ScriptEngine, mpsc::UnboundedReceiver<ClientCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let engine = ScriptEngine::new(tx, NetworkId(1), "testuser".into(), "#test".into());
        (engine, rx)
    }

    #[test]
    fn eval_arithmetic() {
        let (mut engine, _rx) = test_engine();
        // Rhai's `run` returns () — use eval_expression to check the value.
        let result: i64 = engine
            .engine
            .eval_with_scope(&mut engine.scope, "1 + 1")
            .expect("eval should succeed");
        assert_eq!(result, 2);
    }

    #[test]
    fn alias_round_trip() {
        let (mut engine, mut rx) = test_engine();

        // Register an alias that sends a message using the args variable.
        engine
            .register_alias("greet", r##"send_msg("#test", args)"##)
            .expect("register_alias should succeed");

        engine
            .run_alias("greet", "hello world")
            .expect("run_alias should succeed");

        // The alias should have enqueued a PRIVMSG command.
        let cmd = rx.try_recv().expect("should have a command");
        match cmd {
            ClientCommand::SendPrivmsg { target, text, .. } => {
                assert_eq!(target.as_ref(), b"#test");
                assert_eq!(text.as_ref(), b"hello world");
            }
            other => panic!("expected SendPrivmsg, got {other:?}"),
        }
    }

    #[test]
    fn operations_limit_prevents_infinite_loop() {
        let (mut engine, _rx) = test_engine();
        let result = engine.eval("loop {}");
        assert!(result.is_err(), "infinite loop should trigger an error");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Too many operations"),
            "error should mention operations limit, got: {msg}"
        );
    }

    #[test]
    fn nick_and_channel_accessors() {
        let (mut engine, _rx) = test_engine();
        let nick: String = engine
            .engine
            .eval_with_scope(&mut engine.scope, "nick()")
            .expect("nick() should work");
        assert_eq!(nick, "testuser");

        let chan: String = engine
            .engine
            .eval_with_scope(&mut engine.scope, "channel()")
            .expect("channel() should work");
        assert_eq!(chan, "#test");
    }

    #[test]
    fn event_hook_fires() {
        let (mut engine, mut rx) = test_engine();

        // Define a handler function in the scope, then register it as a hook.
        engine
            .eval(r#"fn on_join(ch) { send_msg(ch, "joined!"); }"#)
            .expect("defining handler should work");

        engine
            .register_hook("join", "on_join")
            .expect("register_hook should succeed");

        engine
            .fire_event("join", &[Dynamic::from("#general".to_owned())])
            .expect("fire_event should succeed");

        let cmd = rx.try_recv().expect("should have a command");
        match cmd {
            ClientCommand::SendPrivmsg { target, text, .. } => {
                assert_eq!(target.as_ref(), b"#general");
                assert_eq!(text.as_ref(), b"joined!");
            }
            other => panic!("expected SendPrivmsg, got {other:?}"),
        }
    }
}
