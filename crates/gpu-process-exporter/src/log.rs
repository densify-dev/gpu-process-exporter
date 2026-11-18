// SPDX-License-Identifier: Apache-2.0

//! Minimal timestamped logging to stderr. Log text is not part of the parity contract.

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("{} {}", ::jiff::Timestamp::now().strftime("%Y/%m/%d %H:%M:%S"), format_args!($($arg)*))
    };
}
