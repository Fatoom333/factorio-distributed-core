//! Серверное ядро Factorio Distributed: библиотека.
//!
//! - [`world`] — модель мира M0 (ленты, сундуки, манипуляторы), совпадающая с движком
//!   Factorio потиково;
//! - [`fidelity`] — сверка модели с эталоном, снятым из игры.

pub mod fidelity;
pub mod fuzz;
pub mod world;
