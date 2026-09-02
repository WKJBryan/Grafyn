fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|_| {
            eprintln!("Grafyn E2E runtime could not initialize.");
            std::process::exit(1);
        });
    if runtime
        .block_on(grafyn_lib::test_runtime::run_from_env())
        .is_err()
    {
        eprintln!("Grafyn E2E runtime stopped before it became available.");
        std::process::exit(1);
    }
}
