use crate::actors::{Archivist, VideoAggregator};
use crate::server::start_server;
use crate::utils::config::get_config;
use crate::File;

use tokio::sync::mpsc::unbounded_channel;

use ipfs_api::IpfsClient;

use linked_data::config::Configuration;

pub async fn start_file(_file: File) {
    let ipfs = IpfsClient::default();

    if ipfs.id(None).await.is_err() {
        eprintln!("IPFS must be started beforehand. Aborting...");
        return;
    }

    println!("Initialization...");

    let config = get_config().await;

    let Configuration {
        input_socket_addr,
        mut archive,
        mut video,
        chat,
    } = config;

    let mut handles = Vec::with_capacity(4);

    let (archive_tx, archive_rx) = unbounded_channel();

    archive.archive_live_chat = false;

    let mut archivist = Archivist::new(ipfs.clone(), archive_rx, archive);

    let archive_handle = tokio::spawn(async move {
        archivist.start().await;
    });

    handles.push(archive_handle);

    let (video_tx, video_rx) = unbounded_channel();

    video.pubsub_enable = false;

    let mut video = VideoAggregator::new(ipfs.clone(), video_rx, Some(archive_tx.clone()), video);

    let video_handle = tokio::spawn(async move {
        video.start().await;
    });

    handles.push(video_handle);

    let server_handle = tokio::spawn(async move {
        start_server(
            input_socket_addr,
            video_tx,
            Some(archive_tx),
            ipfs,
            chat.pubsub_topic,
        )
        .await;
    });

    handles.push(server_handle);

    for handle in handles {
        if let Err(e) = handle.await {
            eprintln!("Main: {}", e);
        }
    }
}
