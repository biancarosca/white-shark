use crate::trader::constants::{LEVEL_1_CONTRACTS, LEVEL_2_CONTRACTS};

pub fn get_contract_size(price: f64) -> u64 {
    if price >= 0.60 {
        LEVEL_1_CONTRACTS
    } else {
        LEVEL_2_CONTRACTS
    }
}