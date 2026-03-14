const LEVEL_1_CONTRACTS: u64 = 7;
const LEVEL_2_CONTRACTS: u64 = 14;

pub fn get_contract_size(price: f64) -> u64 {
    if price >= 0.60 {
        LEVEL_1_CONTRACTS
    } else {
        LEVEL_2_CONTRACTS
    }
}
