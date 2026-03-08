use async_proto::Protocol;

pub(crate) const PATH: &str = "/home/fenhl/.local/share/fidera/sock-status";

#[derive(clap::Subcommand, Protocol)]
pub(crate) enum Subcommand {
    BuildWerewolfWeb,
}
