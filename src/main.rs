use white_shark::app::run;
use white_shark::config::Config;
use white_shark::error::Result;
use white_shark::logging::init;

#[tokio::main]
async fn main() -> Result<()> {
    init();

    let config = Config::from_env()?;

    run(config).await
}
