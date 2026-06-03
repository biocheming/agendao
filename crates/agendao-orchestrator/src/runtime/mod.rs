pub mod bridges;
pub mod events;
pub mod loop_impl;
pub mod normalizer;
pub mod policy;
pub mod simple_model_caller;
pub mod stream_text;
pub mod traits;

pub use events::*;
pub use loop_impl::run_loop;
pub use normalizer::normalize;
pub use policy::*;
pub use simple_model_caller::{SimpleModelCaller, SimpleModelCallerConfig};
pub use stream_text::collect_text_chunks;
pub use traits::*;

#[cfg(test)]
mod golden_tests;
