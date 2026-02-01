use sea_orm::entity::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "market_info")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    
    pub ticker: String,
    
    pub timestamp: DateTime<Utc>,
    
    #[sea_orm(nullable)]
    pub strike_price: Option<Decimal>,
    
    pub result: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

