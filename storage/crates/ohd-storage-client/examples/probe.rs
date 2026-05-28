//! Throwaway transport probe: hit a live storage server and print the exact
//! RemoteError. With a bogus token a working transport returns Auth
//! (UNAUTHENTICATED); a Transport error means the client can't even connect.
//! Usage: cargo run --example probe -- https://storage.ohd.dev [token]

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "https://storage.ohd.dev".to_string());
    let token = args.next().unwrap_or_else(|| "ohds_bogus_probe_token".to_string());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let client = match ohd_storage_client::OhdcRemoteClient::new(&url, &token) {
            Ok(c) => c,
            Err(e) => {
                println!("CONSTRUCT ERROR: {e:?}");
                return;
            }
        };

        println!("--- health ---");
        match client.health().await {
            Ok(h) => println!("health OK: {h:?}"),
            Err(e) => println!("health ERR: {e:?}"),
        }

        println!("--- whoami ---");
        match client.whoami().await {
            Ok(w) => println!("whoami OK: {w:?}"),
            Err(e) => println!("whoami ERR: {e:?}"),
        }
    });
}
