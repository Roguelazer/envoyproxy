use clap::Parser;
use std::time::Duration;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[arg(short, long, default_value = "3112", env = "PORT")]
    pub port: u16,
    #[arg(long, default_value = "https://envoy.local", env = "ENVOY_URL")]
    pub envoy_url: url::Url,
    #[arg(long, env = "ENVOY_JWT")]
    pub envoy_jwt: String,
    #[arg(
        long,
        default_value = "60",
        help = "Interval to poll the system state, in seconds"
    )]
    pub poll_interval_secs: u32,
    #[arg(
        long,
        default_value = "21600",
        help = "Interval to collect system inventory, in seconds"
    )]
    pub inventory_poll_interval_secs: u32,
}

impl Args {
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs as u64)
    }

    pub fn inventory_poll_interval(&self) -> Duration {
        Duration::from_secs(self.inventory_poll_interval_secs as u64)
    }
}
