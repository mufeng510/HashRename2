//! 核心业务模块(平台无关,三端共用,需求 §3)。

pub mod duplicate_detector;
pub mod error;
pub mod hasher;
pub mod lock;
pub mod models;
pub mod processor;
pub mod progress;
pub mod rename_planner;
pub mod renamer;
pub mod scanner;
pub mod sorter;
pub mod trash;
