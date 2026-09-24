// `#[contracterror]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_common::impl_display_error;
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AirdropError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    RootNotSet = 4,
    InvalidProof = 5,
    AlreadyClaimed = 6,
    InvalidAmount = 7,
    ClaimWindowClosed = 8,
}

impl_display_error!(
    AirdropError,
    AlreadyInitialized => "already initialized",
    NotInitialized     => "not initialized",
    Unauthorized       => "not authorized",
    RootNotSet         => "merkle root not set",
    InvalidProof       => "invalid merkle proof",
    AlreadyClaimed     => "already claimed",
    InvalidAmount      => "invalid amount",
    ClaimWindowClosed  => "claim window closed",
);

#[cfg(test)]
mod tests {
    extern crate std;

    use super::AirdropError;
    use std::format;
    use std::string::String;

    #[allow(clippy::as_conversions)]
    fn render_error_code_snapshot() -> String {
        format!(
            "\
AirdropError::AlreadyInitialized = {}\n\
AirdropError::NotInitialized = {}\n\
AirdropError::Unauthorized = {}\n\
AirdropError::RootNotSet = {}\n\
AirdropError::InvalidProof = {}\n\
AirdropError::AlreadyClaimed = {}\n\
AirdropError::InvalidAmount = {}\n\
AirdropError::ClaimWindowClosed = {}\n",
            AirdropError::AlreadyInitialized as u32,
            AirdropError::NotInitialized as u32,
            AirdropError::Unauthorized as u32,
            AirdropError::RootNotSet as u32,
            AirdropError::InvalidProof as u32,
            AirdropError::AlreadyClaimed as u32,
            AirdropError::InvalidAmount as u32,
            AirdropError::ClaimWindowClosed as u32,
        )
    }

    #[test]
    fn airdrop_error_codes_match_snapshot() {
        assert_eq!(
            render_error_code_snapshot(),
            include_str!("../snapshots/error_codes.snap")
        );
    }
}
