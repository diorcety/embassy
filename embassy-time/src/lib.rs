#![cfg_attr(not(any(feature = "std", feature = "wasm", test)), no_std)]
#![allow(async_fn_in_trait)]
#![doc = include_str!("../README.md")]
#![allow(clippy::new_without_default)]
#![warn(missing_docs)]

//! ## Feature flags
#![doc = document_features::document_features!(feature_label = r#"<span class="stab portability"><code>{feature}</code></span>"#)]

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

mod delay;
mod duration;
mod instant;
mod timer;

#[cfg(feature = "mock-driver")]
mod driver_mock;

#[cfg(feature = "mock-driver")]
pub use driver_mock::MockDriver;

#[cfg(feature = "std")]
mod driver_std;
#[cfg(feature = "wasm")]
mod driver_wasm;

pub use delay::{block_for, Delay};
pub use duration::Duration;
pub use duration::DurationType;
pub use embassy_time_driver::TICK_HZ;
pub use embassy_time_driver::TickType;
pub use instant::Instant;
pub use timer::{with_deadline, with_timeout, Ticker, TimeoutError, Timer, WithTimeout};

const fn gcd(a: DurationType, b: DurationType) -> DurationType {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

pub(crate) const GCD_1K: DurationType = gcd(TICK_HZ, 1_000);
pub(crate) const GCD_1M: DurationType = gcd(TICK_HZ, 1_000_000);
pub(crate) const GCD_1G: DurationType = gcd(TICK_HZ, 1_000_000_000);

#[cfg(all(feature = "defmt-timestamp-uptime-s", feature = "duration-u32"))]
defmt::timestamp! {"{=u32}", Instant::now().as_secs()}

#[cfg(all(feature = "defmt-timestamp-uptime-s", not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64}", Instant::now().as_secs() }

#[cfg(all(feature = "defmt-timestamp-uptime-ms", feature = "duration-u32"))]
defmt::timestamp! {"{=u32:ms}", Instant::now().as_millis()}

#[cfg(all(feature = "defmt-timestamp-uptime-ms", not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64:ms}", Instant::now().as_millis() }

#[cfg(all(any(feature = "defmt-timestamp-uptime", feature = "defmt-timestamp-uptime-us"), feature = "duration-u32"))]
defmt::timestamp! {"{=u32:us}", Instant::now().as_micros()}

#[cfg(all(any(feature = "defmt-timestamp-uptime", feature = "defmt-timestamp-uptime-us"), not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64:us}", Instant::now().as_micros() }

#[cfg(all(feature = "defmt-timestamp-uptime-ts", feature = "duration-u32"))]
defmt::timestamp! {"{=u32:ts}", Instant::now().as_secs()}

#[cfg(all(feature = "defmt-timestamp-uptime-ts", not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64:ts}", Instant::now().as_secs() }

#[cfg(all(feature = "defmt-timestamp-uptime-tms", feature = "duration-u32"))]
defmt::timestamp! {"{=u32:tms}", Instant::now().as_millis()}

#[cfg(all(feature = "defmt-timestamp-uptime-tms", not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64:tms}", Instant::now().as_millis() }

#[cfg(all(feature = "defmt-timestamp-uptime-tus", feature = "duration-u32"))]
defmt::timestamp! {"{=u32:tus}", Instant::now().as_micros() }

#[cfg(all(feature = "defmt-timestamp-uptime-tus", not(feature = "duration-u32")))]
defmt::timestamp! {"{=u64:tus}", Instant::now().as_micros() }

