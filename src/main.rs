use terraintile::server;

fn main() -> anyhow::Result<()> {
    let mut host = "0.0.0.0".to_string();
    let mut port: u16 = 8080;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = args.next().unwrap_or(host),
            "--port" => {
                port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--port trenger et tall"))?;
            }
            "--help" | "-h" => {
                println!("terraintile [--host ADDR] [--port PORT]   (standard 0.0.0.0:8080)");
                return Ok(());
            }
            other => anyhow::bail!("ukjent argument: {other} (bruk --host/--port)"),
        }
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(server::serve(host, port))
}
