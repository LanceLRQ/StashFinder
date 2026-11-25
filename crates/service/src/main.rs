fn main() -> anyhow::Result<()> {
    stashfinder_service::runtime::run(&std::env::args().skip(1).collect::<Vec<_>>())
}
