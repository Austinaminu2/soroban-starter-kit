// `#[contracterror]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_common::impl_display_error;
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, Debug)]
pub enum TimelockError {
    NotAuthorized = 1,
    AlreadyInitialized = 2,
    NotInitialized = 3,
    NotYetReleasable = 4,
    AlreadyReleased = 5,
    AlreadyCancelled = 6,
    InvalidAmount = 7,
    InvalidReleaseLedger = 8,
}

impl_display_error!(
    TimelockError,
    NotAuthorized       => "not authorized",
    AlreadyInitialized  => "already initialized",
    NotInitialized      => "not initialized",
    NotYetReleasable    => "not yet releasable",
    AlreadyReleased     => "already released",
    AlreadyCancelled    => "already cancelled",
    InvalidAmount       => "invalid amount",
    InvalidReleaseLedger => "invalid release ledger",
);

#[cfg(test)]
mod tests {
    extern crate std;

    use super::TimelockError;
    use std::format;
    use std::string::String;

    #[allow(clippy::as_conversions)]
    fn render_error_code_snapshot() -> String {
        format!(
            "\
TimelockError::NotAuthorized = {}\n\
TimelockError::AlreadyInitialized = {}\n\
TimelockError::NotInitialized = {}\n\
TimelockError::NotYetReleasable = {}\n\
TimelockError::AlreadyReleased = {}\n\
TimelockError::AlreadyCancelled = {}\n\
TimelockError::InvalidAmount = {}\n\
TimelockError::InvalidReleaseLedger = {}\n",
            TimelockError::NotAuthorized as u32,
            TimelockError::AlreadyInitialized as u32,
            TimelockError::NotInitialized as u32,
            TimelockError::NotYetReleasable as u32,
            TimelockError::AlreadyReleased as u32,
            TimelockError::AlreadyCancelled as u32,
            TimelockError::InvalidAmount as u32,
            TimelockError::InvalidReleaseLedger as u32,
        )
    }

    #[test]
    fn timelock_error_codes_match_snapshot() {
        assert_eq!(
            render_error_code_snapshot(),
            include_str!("../snapshots/error_codes.snap")
        );
    }
}
