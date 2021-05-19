use std::collections::HashMap;

use crate::app::ENS_NAME;
use crate::components::{Navbar, VideoThumbnail};
use crate::utils::ipfs::IpfsService;
use crate::utils::local_storage::{get_cid, get_local_storage, set_cid, set_local_beacon};
use crate::utils::web3::Web3Service;

use wasm_bindgen_futures::spawn_local;

use web_sys::Storage;

use yew::prelude::{html, Component, ComponentLink, Html, Properties, ShouldRender};
use yew::services::ConsoleService;

use linked_data::beacon::Beacon;
use linked_data::video::{VideoList, VideoMetadata};

use cid::Cid;

use reqwest::Error;

pub struct Videos {
    link: ComponentLink<Self>,

    ipfs: IpfsService,

    storage: Option<Storage>,

    beacon_cid: Option<Cid>,
    beacon: Option<Beacon>,

    searching: bool,

    list_cid: Option<Cid>,
    video_list: Option<VideoList>,

    call_count: usize,
    metadata_map: HashMap<Cid, VideoMetadata>,
}

pub enum Msg {
    ResolveName(Result<Cid, web3::contract::Error>),
    Beacon(Result<Beacon, Error>),
    List((Cid, Result<VideoList, Error>)),
    ResolveList(Result<(Cid, VideoList), Error>),
    Metadata((Cid, Result<VideoMetadata, Error>)),
}

#[derive(Properties, Clone)]
pub struct Props {
    pub ipfs: IpfsService, // From app.
    pub web3: Web3Service, // From app.
}

impl Component for Videos {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let Props { ipfs, web3 } = props;

        let window = web_sys::window().expect("Can't get window");
        let storage = get_local_storage(&window);

        let beacon_cid = get_cid(ENS_NAME, storage.as_ref());

        if let Some(cid) = beacon_cid {
            let cb = link.callback_once(Msg::Beacon);
            let client = ipfs.clone();

            spawn_local(async move { cb.emit(client.dag_get(cid, Option::<String>::None).await) });
        }

        // Check for beacon updates by resolving name.
        let cb = link.callback_once(Msg::ResolveName);
        let name = ENS_NAME.to_owned();

        spawn_local(async move { cb.emit(web3.get_ipfs_content(name).await) });

        Self {
            link,
            ipfs,
            beacon_cid,
            beacon: None,
            searching: true,
            list_cid: None,
            video_list: None,
            storage,
            call_count: 0,
            metadata_map: HashMap::with_capacity(10),
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::ResolveName(result) => self.on_name_resolved(result),
            Msg::Beacon(result) => self.on_beacon_update(result),
            Msg::List((cid, result)) => self.on_video_list_update(cid, result),
            Msg::ResolveList(result) => self.on_video_list_resolved(result),
            Msg::Metadata((cid, result)) => self.on_video_metadata_update(cid, result),
        }
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        let content = if self.searching {
            html! { <div class="center_text">  {"Loading..."} </div> }
        } else {
            let video_list = self.video_list.as_ref().unwrap();

            html! {
                <div class="video_list">
                {
                    for video_list.metadata.iter().rev().map(|ipld| {
                        let cid = ipld.link;
                        let mt = &self.metadata_map[&cid];
                        html! {
                            <VideoThumbnail metadata_cid=cid metadata=mt />
                        }
                    }
                    )
                }
                </div>
            }
        };

        html! {
            <div class="videos_page">
                <Navbar />
                { content }
            </div>
        }
    }
}

impl Videos {
    /// Callback when Ethereum Name Service resolve name to beacon Cid.
    fn on_name_resolved(&mut self, res: Result<Cid, web3::contract::Error>) -> bool {
        let cid = match res {
            Ok(cid) => cid,
            Err(e) => {
                ConsoleService::error(&format!("{:?}", e));
                return false;
            }
        };

        if let Some(beacon_cid) = self.beacon_cid.as_ref() {
            if *beacon_cid == cid {
                return false;
            }
        }

        let cb = self.link.callback_once(Msg::Beacon);
        let client = self.ipfs.clone();

        spawn_local(async move { cb.emit(client.dag_get(cid, Option::<String>::None).await) });

        #[cfg(debug_assertions)]
        ConsoleService::info("Name Update");

        set_local_beacon(ENS_NAME, &cid, self.storage.as_ref());

        self.beacon_cid = Some(cid);

        false
    }

    /// Callback when IPFS dag get return beacon node.
    fn on_beacon_update(&mut self, res: Result<Beacon, Error>) -> bool {
        let beacon = match res {
            Ok(b) => b,
            Err(e) => {
                ConsoleService::error(&format!("{:?}", e));
                return false;
            }
        };

        #[cfg(debug_assertions)]
        ConsoleService::info("Beacon Update");

        if let Some(cid) = get_cid(&beacon.videos, self.storage.as_ref()) {
            self.list_cid = Some(cid);

            let cb = self.link.callback_once(Msg::List);
            let client = self.ipfs.clone();

            spawn_local(async move {
                cb.emit((cid, client.dag_get(cid, Option::<String>::None).await))
            });
        }

        let cb = self.link.callback_once(Msg::ResolveList);
        let client = self.ipfs.clone();
        let ipns = beacon.videos.clone();

        spawn_local(async move { cb.emit(client.resolve_and_dag_get(ipns).await) });

        self.beacon = Some(beacon);

        false
    }

    /// Callback when IPFS dag get return VideoList node.
    fn on_video_list_resolved(&mut self, res: Result<(Cid, VideoList), Error>) -> bool {
        let (cid, list) = match res {
            Ok((cid, list)) => (cid, list),
            Err(e) => {
                ConsoleService::error(&format!("{:?}", e));
                return false;
            }
        };

        self.on_video_list_update(cid, Ok(list))
    }

    /// Callback when IPFS resolve and dag get VideoList node.
    fn on_video_list_update(&mut self, list_cid: Cid, res: Result<VideoList, Error>) -> bool {
        let list = match res {
            Ok(l) => l,
            Err(e) => {
                ConsoleService::error(&format!("{:?}", e));
                return false;
            }
        };

        if let Some(old_list_cid) = self.list_cid.as_ref() {
            if *old_list_cid == list_cid && self.video_list.is_some() {
                return false;
            }
        }

        let beacon = match self.beacon.as_ref() {
            Some(b) => b,
            None => return false,
        };

        #[cfg(debug_assertions)]
        ConsoleService::info("Video List Update");

        if let Some(old_list_cid) = self.list_cid.as_ref() {
            if *old_list_cid != list_cid {
                self.list_cid = Some(list_cid);
                set_cid(&beacon.videos, &list_cid, self.storage.as_ref());
            }
        } else {
            self.list_cid = Some(list_cid);
            set_cid(&beacon.videos, &list_cid, self.storage.as_ref());
        }

        if list.metadata.is_empty() {
            self.video_list = Some(list);
            self.searching = false;
            return true;
        }

        for metadata in list.metadata.iter().rev() {
            let cb = self.link.callback_once(Msg::Metadata);
            let client = self.ipfs.clone();
            let cid = metadata.link;

            spawn_local(async move {
                cb.emit((cid, client.dag_get(cid, Option::<String>::None).await))
            });
        }

        self.call_count += list.metadata.len();

        self.video_list = Some(list);
        self.searching = false;

        false
    }

    /// Callback when IPFS dag get returns VideoMetadata node.
    fn on_video_metadata_update(&mut self, cid: Cid, res: Result<VideoMetadata, Error>) -> bool {
        let metadata = match res {
            Ok(d) => d,
            Err(e) => {
                ConsoleService::error(&format!("{:?}", e));
                return false;
            }
        };

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!(
            "Display Add => {} \n {}",
            &cid.to_string(),
            &serde_json::to_string_pretty(&metadata).expect("Can't print")
        ));

        self.metadata_map.insert(cid, metadata);

        if self.call_count > 0 {
            self.call_count -= 1;
        }

        if self.call_count == 0 {
            #[cfg(debug_assertions)]
            ConsoleService::info("Refresh");

            return true;
        }

        false
    }
}