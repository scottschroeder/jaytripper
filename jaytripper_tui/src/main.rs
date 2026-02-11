mod cli;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cmd = cli::get_args();
    setup_logger(cmd.verbose());
    log::trace!("Args: {:?}", cmd);

    cmd.run().await.map_err(|error| {
        log::error!("{:?}", error);
        anyhow::anyhow!("unrecoverable {} failure", clap::crate_name!())
    })
}

pub(crate) fn setup_logger(level: u8) {
    let mut builder = pretty_env_logger::formatted_timed_builder();

    let log_level = match level {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    builder.filter_level(log_level);
    builder.format_timestamp_millis();
    builder.init();
}
