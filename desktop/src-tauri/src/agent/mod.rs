//! The agentic run: provider abstraction, tool loop and the deferred-write
//! approval flow. See `docs/agent-contract.md` for the command/event contract.

pub mod events;
pub mod pricing;
pub mod prompt;
pub mod providers;
pub mod runner;
pub mod tools;
pub mod types;

pub use runner::AgentRegistry;
