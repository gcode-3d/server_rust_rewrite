use std::{fs::File, io::Write, path::Path};

use async_recursion::async_recursion;
use hyper::{
    body::{Buf, HttpBody},
    Client, Uri,
};
use hyper_tls::HttpsConnector;
use serde_json::Value;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn check_updates() {
    let current_id = get_current_build_info();
    let latest_info = get_release_info().await;
    let latest_info = latest_info.unwrap();
    if current_id.eq(&latest_info.id) {
        return println!("[CLIENT][UPDATE] Client up to date, using: {}", current_id);
    }
    println!("[CLIENT][UPDATE] New version available. Downloading...");
    download_release(latest_info.url).await;
    update_build_info(current_id, latest_info.id);
}

fn update_build_info(old: String, new: String) {
    let mut file = File::create(Path::new("./client/build_info"))
        .expect("[CLIENT][DOWNLOAD][ERROR] Cannot update build info");

    file.write(new.as_bytes())
        .expect("[CLIENT][UPDATE][ERROR] Cannot update build info");

    println!("[CLIENT][UPDATE] Client updated from {} -> {}", old, new);
}

/*
    Open the buildinfo file, and read the current build id from it.
*/
fn get_current_build_info() -> String {
    return match std::fs::read_to_string("./client/build_info") {
        Ok(file) => file,
        Err(_) => "".to_string(),
    };
}
/*
    Download the release info from github
*/
async fn get_release_info() -> Result<ClientInfo> {
    let client = Client::builder().build::<_, hyper::Body>(HttpsConnector::new());
    let request = hyper::Request::builder()
        .uri("https://api.github.com/repos/gcode-3d/client/releases/latest")
        .header("User-Agent", "gcode-3d")
        .body(hyper::Body::empty())
        .expect("[CLIENT][UPDATE][ERROR] Cannot create request");
    let res = client.request(request).await?;

    let json: Value = serde_json::from_reader(hyper::body::aggregate(res).await?.reader())?;

    let releases = json
        .get("assets")
        .and_then(|value| value.as_array())
        .expect("[CLIENT][UPDATE][ERROR] Cannot fetch releases");

    if releases.len() != 1 {
        panic!("[CLIENT][UPDATE][ERROR] Release count does not match expected amount");
    }
    let release = &releases.clone()[0];

    let id = release
        .get("id")
        .expect("[CLIENT][UPDATE][ERROR] No comparable id found for release")
        .as_u64()
        .expect("[CLIENT][UPDATE][ERROR] No comparable id found for release")
        .to_string();
    let url = release
        .get("browser_download_url")
        .expect("[CLIENT][UPDATE][ERROR] No download url found for release")
        .as_str()
        .expect("[CLIENT][UPDATE][ERROR] No download url found for release")
        .to_string();

    return Ok(ClientInfo { id, url });
}

/*
    Download a release, and temporarily store it in memory.
*/
#[async_recursion]
async fn download_release(url: String) {
    let result = std::fs::create_dir("./client/");
    if result.is_err() {
        let err = result.unwrap_err();
        if (format!("{:?}", err.kind()) != "AlreadyExists".to_string()) {
            panic!("{}", err);
        }
    }
    let result = std::fs::remove_file(Path::new("./client/dist.zip"));
    if result.is_err() {
        let err = result.unwrap_err();
        if (format!("{:?}", err.kind()) != "NotFound".to_string()) {
            panic!("{}", err);
        }
    }

    let client = Client::builder().build::<_, hyper::Body>(HttpsConnector::new());
    let res = client.get(url.parse::<Uri>().unwrap()).await;
    if res.is_err() {
        panic!("[CLIENT][DOWNLOAD][ERROR] {}", res.unwrap_err());
    }
    let mut res = res.unwrap();
    if res.status() == 301 || res.status() == 302 {
        return download_release(
            res.headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
        )
        .await;
    }
    let mut file = File::create(Path::new("./client/dist.zip"))
        .expect("[CLIENT][DOWNLOAD][ERROR] Cannot download zip file");

    while let Some(chunk) = res.data().await {
        if chunk.is_err() {
            panic!("[CLIENT][DOWNLOAD][ERROR] {}", chunk.unwrap_err());
        }
        file.write(&chunk.unwrap())
            .expect("[CLIENT][DOWNLOAD][ERROR] Cannot write zip file chunk");
    }
    drop(file);
    let file =
        File::open("./client/dist.zip").expect("[CLIENT][UPDATE][ERROR] Cannot open zip file");

    let mut archive =
        zip::ZipArchive::new(file).expect("[CLIENT][UPDATE][ERROR] Cannot open zip file");

    archive
        .extract("./client")
        .expect("[CLIENT][UPDATE][ERROR] Cannot extract zip file");

    return ();
}

#[derive(Debug)]
struct ClientInfo {
    id: String,
    url: String,
}
