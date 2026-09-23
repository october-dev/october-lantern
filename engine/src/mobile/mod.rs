//! "Connect to October phone app": Lantern as an October host computer, so the October phone
//! app pairs with it and reaches Lantern's agents through October's relay. See `host.rs`.

pub mod api;
pub mod control;
pub mod frames;
pub mod host;
pub mod noise;
pub mod store;
