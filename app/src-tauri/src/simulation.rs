use caspers_universe::{Result, SimulationBuilder, SiteSetup};
use chrono::Duration;
use url::Url;

pub struct SimulationConfig {
    pub duration: Duration,
    pub snapshot_interval: Duration,
    pub time_increment: Duration,
    pub results_location: Url,
    pub nodes_file_path: String,
    pub edges_file_path: String,
}

pub fn register_location(name: String, site: SiteSetup) {
    // Implementation goes here
}

pub fn run_simulation(config: SimulationConfig) -> Result<()> {
    let mut simulation = SimulationBuilder::new()
        .with_result_storage_location(config.results_location)
        .with_snapshot_interval(Duration::minutes(10))
        .with_time_increment(Duration::minutes(1));

    Ok(())
}
