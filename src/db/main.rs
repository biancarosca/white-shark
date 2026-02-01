use sea_orm::{Database, DatabaseConnection, ActiveValue, EntityTrait};
use sea_query::{Table, ColumnDef, MysqlQueryBuilder, Index, Alias};
use tracing::info;
use chrono::Utc;
use rust_decimal::Decimal;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::db::{market_data, market_info};

pub struct Db {
    connection: DatabaseConnection,
}

impl Db {
    pub async fn new(database_url: &str) -> Result<Self> {
        let connection = Database::connect(database_url)
            .await
            .map_err(|e| Error::Database(format!("Failed to connect to database: {}", e)))?;
        
        info!("✅ Connected to TiDB database");
        Ok(Self { connection })
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    pub async fn create_market_data_table(&self) -> Result<()> {
        info!("Creating market_data table...");
        
        use sea_orm::ConnectionTrait;
        
        let stmt = Table::create()
            .table(Alias::new("market_data"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .auto_increment()
                    .primary_key()
            )
            .col(
                ColumnDef::new(Alias::new("timestamp"))
                    .date_time()
                    .not_null()
            )
            .col(
                ColumnDef::new(Alias::new("ticker"))
                    .string_len(50)
                    .not_null()
            )
            .col(
                ColumnDef::new(Alias::new("yes_ask"))
                    .decimal_len(10, 4)
            )
            .col(
                ColumnDef::new(Alias::new("yes_bid"))
                    .decimal_len(10, 4)
            )
            .col(
                ColumnDef::new(Alias::new("no_ask"))
                    .decimal_len(10, 4)
            )
            .col(
                ColumnDef::new(Alias::new("no_bid"))
                    .decimal_len(10, 4)
            )
            .index(
                Index::create()
                    .name("idx_ticker")
                    .col(Alias::new("ticker"))
            )
            .index(
                Index::create()
                    .name("idx_timestamp")
                    .col(Alias::new("timestamp"))
            )
            .to_owned();
        
        let sql = stmt.to_string(MysqlQueryBuilder);
        
        self.connection.execute_unprepared(&sql)
            .await
            .map_err(|e| Error::Database(format!("Failed to create table: {}", e)))?;
        
        info!("✅ Created market_data table");
        Ok(())
    }

    pub async fn create_market_info_table(&self) -> Result<()> {
        info!("Creating market_info table...");
        
        use sea_orm::ConnectionTrait;
        
        let stmt = Table::create()
            .table(Alias::new("market_info"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .auto_increment()
                    .primary_key()
            )
            .col(
                ColumnDef::new(Alias::new("timestamp"))
                    .date_time()
                    .not_null()
            )
            .col(
                ColumnDef::new(Alias::new("ticker"))
                    .string_len(50)
                    .not_null()
            )
            .col(
                ColumnDef::new(Alias::new("strike_price"))
                    .decimal_len(20, 8)
            )
            .col(
                ColumnDef::new(Alias::new("result"))
                    .string_len(20)
                    .not_null()
            )
            .index(
                Index::create()
                    .name("idx_ticker")
                    .col(Alias::new("ticker"))
            )
            .index(
                Index::create()
                    .name("idx_timestamp")
                    .col(Alias::new("timestamp"))
            )
            .to_owned();
        
        let sql = stmt.to_string(MysqlQueryBuilder);
        
        self.connection.execute_unprepared(&sql)
            .await
            .map_err(|e| Error::Database(format!("Failed to create table: {}", e)))?;
        
        info!("✅ Created market_data table");
        Ok(())
    }

    pub async fn insert_market_data(
        &self,
        ticker: &str,
        timestamp: chrono::DateTime<Utc>,
        yes_ask: f64,
        yes_bid: f64,
        no_ask: f64,
        no_bid: f64,
    ) -> Result<()> {
        let active_model = Self::create_market_data_active_model(
            ticker,
            timestamp,
            yes_ask,
            yes_bid,
            no_ask,
            no_bid,
        );

        <market_data::Entity as EntityTrait>::insert(active_model)
            .exec(&self.connection)
            .await
            .map_err(|e| Error::Database(format!("Failed to insert data: {}", e)))?;

        Ok(())
    }

    pub async fn insert_market_data_batch(
        &self,
        records: Vec<(String, chrono::DateTime<Utc>, f64, f64, f64, f64)>,
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let active_models: Vec<market_data::ActiveModel> = records
            .into_iter()
            .map(|(ticker, timestamp, yes_ask, yes_bid, no_ask, no_bid)| {
                Self::create_market_data_active_model(&ticker, timestamp, yes_ask, yes_bid, no_ask, no_bid)
            })
            .collect();

        <market_data::Entity as EntityTrait>::insert_many(active_models)
            .exec(&self.connection)
            .await
            .map_err(|e| Error::Database(format!("Failed to batch insert data: {}", e)))?;

        Ok(())
    }


    fn create_market_data_active_model(
        ticker: &str,
        timestamp: chrono::DateTime<Utc>,
        yes_ask: f64,
        yes_bid: f64,
        no_ask: f64,
        no_bid: f64,
    ) -> market_data::ActiveModel {
        let to_decimal = |v: f64| -> Option<Decimal> {
            Decimal::from_str(&format!("{:.10}", v)).ok()
        };
        
        market_data::ActiveModel {
            id: ActiveValue::NotSet,
            ticker: ActiveValue::Set(ticker.to_string()),
            timestamp: ActiveValue::Set(timestamp),
            yes_ask: ActiveValue::Set(to_decimal(yes_ask)),
            yes_bid: ActiveValue::Set(to_decimal(yes_bid)),
            no_ask: ActiveValue::Set(to_decimal(no_ask)),
            no_bid: ActiveValue::Set(to_decimal(no_bid)),
        }
    }

    fn create_market_info_active_model(
        ticker: &str,
        timestamp: chrono::DateTime<Utc>,
        strike_price: Option<f64>,
        result: &str,
    ) -> market_info::ActiveModel {
        let to_decimal = |v: f64| -> Option<Decimal> {
            Decimal::from_str(&format!("{:.10}", v)).ok()
        };
    
        market_info::ActiveModel {
            id: ActiveValue::NotSet,
            ticker: ActiveValue::Set(ticker.to_string()),
            timestamp: ActiveValue::Set(timestamp),
            strike_price: ActiveValue::Set(strike_price.and_then(to_decimal)),
            result: ActiveValue::Set(result.to_string().to_uppercase()),
        }
    }

    pub async fn insert_market_info(
        &self,
        ticker: &str,
        timestamp: chrono::DateTime<Utc>,
        strike_price: Option<f64>,
        result: &str,
    ) -> Result<()> {
        let active_model = Self::create_market_info_active_model(
            ticker,
            timestamp,
            strike_price,
            result,
        );
    
        <market_info::Entity as EntityTrait>::insert(active_model)
            .exec(&self.connection)
            .await
            .map_err(|e| Error::Database(format!("Failed to insert market info: {}", e)))?;
    
        Ok(())
    }
}

