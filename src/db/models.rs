use sea_orm::entity::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "market_data")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    
    pub ticker: String,
    
    #[sea_orm(nullable)]
    pub strike_price: Option<Decimal>,
    
    pub timestamp: DateTime<Utc>,
    
    #[sea_orm(nullable)]
    pub yes_ask: Option<Decimal>,
    
    #[sea_orm(nullable)]
    pub yes_bid: Option<Decimal>,
    
    #[sea_orm(nullable)]
    pub no_ask: Option<Decimal>,
    
    #[sea_orm(nullable)]
    pub no_bid: Option<Decimal>,
    
    #[sea_orm(nullable)]
    pub price: Option<Decimal>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

