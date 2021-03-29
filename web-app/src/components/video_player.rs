use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;

use crate::utils::bindings::{ipfs_subscribe, ipfs_unsubscribe};
use crate::utils::ema::ExponentialMovingAverage;
use crate::utils::ipfs::{
    audio_video_cat, init_cat, ipfs_dag_get_callback, ipfs_dag_get_path_callback,
};

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use web_sys::{HtmlMediaElement, MediaSource, MediaSourceReadyState, SourceBuffer, Url, Window};

use js_sys::Uint8Array;

use yew::prelude::{html, Component, ComponentLink, Html, Properties, ShouldRender};
use yew::services::ConsoleService;

use linked_data::video::{SetupNode, TempSetupNode, Track, VideoMetadata, VideoNode};

use cid::Cid;

const FORWARD_BUFFER_LENGTH: f64 = 16.0;
const BACK_BUFFER_LENGTH: f64 = 8.0;

const SETUP_PATH: &str = "/time/hour/0/minute/0/second/0/video/setup/";

enum MachineState {
    Load,
    Switch,
    Flush,
    Timeout,
    AdaptativeBitrate,
    Status,
}

struct MediaBuffers {
    audio: SourceBuffer,
    video: SourceBuffer,

    tracks: Vec<Track>,
}

struct LiveStream {
    topic: String,
    streamer_peer_id: String,

    previous: Option<Cid>,
    buffer: VecDeque<(Cid, VideoNode)>,
    unordered_buffer: HashMap<Cid, VideoNode>,

    pubsub_closure: Closure<dyn Fn(String, Vec<u8>)>,
}

pub struct VideoPlayer {
    link: ComponentLink<Self>,

    metadata: Option<VideoMetadata>,
    live_stream: Option<LiveStream>,

    window: Window,
    media_element: Option<HtmlMediaElement>,
    media_source: MediaSource,
    media_buffers: Option<MediaBuffers>,
    object_url: String,
    poster_link: String,

    /// Level >= 1 since 0 is audio
    level: usize,
    state: MachineState,
    ema: ExponentialMovingAverage,

    source_open_closure: Option<Closure<dyn Fn()>>,
    seeking_closure: Option<Closure<dyn Fn()>>,
    update_end_closure: Option<Closure<dyn Fn()>>,
    timeout_closure: Option<Closure<dyn Fn()>>,
    handle: i32,
}

pub enum Msg {
    SourceOpen,
    Seeking,
    UpdateEnd,
    Timeout,
    SetupNode(SetupNode),
    Append((Option<Uint8Array>, Uint8Array)),
    PubSub((String, Vec<u8>)),
    VideoNode((Cid, VideoNode)),
}

#[derive(Clone, Properties)]
pub struct Props {
    pub metadata: Option<VideoMetadata>,
    pub topic: Option<String>,
    pub streamer_peer_id: Option<String>,
}

impl Component for VideoPlayer {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let Props {
            metadata,
            topic,
            streamer_peer_id,
        } = props;

        let window = web_sys::window().expect("Can't get window");

        let ema = ExponentialMovingAverage::new(&window);

        let media_source = MediaSource::new().expect("Can't create media source");

        let object_url = Url::create_object_url_with_source(&media_source)
            .expect("Can't create url from source");

        let mut poster_link = String::from("ipfs://");
        //TODO
        //poster_link.push_str(&metadata.image.link.to_string());

        let cb = link.callback_once(|_| Msg::SourceOpen);
        let closure = Closure::wrap(Box::new(move || cb.emit(())) as Box<dyn Fn()>);
        media_source.set_onsourceopen(Some(closure.as_ref().unchecked_ref()));
        let source_open_closure = Some(closure);

        let live_stream = match topic {
            Some(topic) => {
                let cb = link.callback(Msg::PubSub);
                let pubsub_closure =
                    Closure::wrap(
                        Box::new(move |from: String, data: Vec<u8>| cb.emit((from, data)))
                            as Box<dyn Fn(String, Vec<u8>)>,
                    );
                ipfs_subscribe(&topic, closure.as_ref().unchecked_ref());

                Some(LiveStream {
                    topic,
                    streamer_peer_id: streamer_peer_id.unwrap(),
                    previous: None,
                    buffer: VecDeque::with_capacity(5),
                    unordered_buffer: HashMap::with_capacity(5),
                    pubsub_closure,
                })
            }
            None => None,
        };

        Self {
            link,

            metadata,
            live_stream,

            window,
            media_element: None,
            media_source,
            media_buffers: None,
            object_url,
            poster_link,

            level: 1, // start at 1 since 0 is audio
            state: MachineState::Timeout,
            ema,

            source_open_closure,
            seeking_closure: None,
            update_end_closure: None,
            timeout_closure: None,
            handle: 0,
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::SourceOpen => self.on_source_open(),
            Msg::Seeking => self.on_seeking(),
            Msg::UpdateEnd => self.on_update_end(),
            Msg::Timeout => self.on_timeout(),
            Msg::SetupNode(node) => self.add_source_buffer(node),
            Msg::Append((aud_res, vid_res)) => self.append_buffers(aud_res, vid_res),
            Msg::PubSub((from, data)) => self.on_pubsub_update(from, data),
            Msg::VideoNode((cid, node)) => self.buffer_video_node(cid, node),
        }

        false
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            <video class="video_player" id="video_player" autoplay=true controls=true poster=self.poster_link />
        }
    }

    fn rendered(&mut self, first_render: bool) {
        if first_render {
            let document = self.window.document().expect("Can't get document");

            let media_element: HtmlMediaElement = document
                .get_element_by_id("video_player")
                .expect("No element with this Id")
                .dyn_into()
                .expect("Not Media Element");

            media_element.set_src(&self.object_url);
            self.media_element = Some(media_element);

            self.seeking_closure = match self.metadata {
                Some(metadata) => {
                    let cb = self.link.callback(|_| Msg::Seeking);
                    let closure = Closure::wrap(Box::new(move || cb.emit(())) as Box<dyn Fn()>);
                    media_element.set_onseeking(Some(closure.as_ref().unchecked_ref()));

                    Some(closure)
                }
                None => None,
            };
        }
    }

    fn destroy(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("Dropping VideoPlayer");

        if let Some(live) = self.live_stream.as_ref() {
            ipfs_unsubscribe(&live.topic);
        }

        if self.handle != 0 {
            self.window.clear_timeout_with_handle(self.handle);
        }
    }
}

impl VideoPlayer {
    /// Callback when MediaSource is linked to video element.
    fn on_source_open(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("On Source Open");

        self.media_source.set_onsourceopen(None);
        self.source_open_closure = None;

        if let Some(metadata) = self.metadata {
            self.media_source.set_duration(metadata.duration);

            let cid = metadata.video.link;

            let cb = self.link.callback_once(Msg::SetupNode);

            spawn_local(ipfs_dag_get_path_callback::<_, TempSetupNode, SetupNode>(
                cid, SETUP_PATH, cb,
            ));
        }
    }

    /// Callback when GossipSub receive an update.
    fn on_pubsub_update(&mut self, from: String, data: Vec<u8>) {
        let live = self.live_stream.expect("Not Live Stream");

        #[cfg(debug_assertions)]
        ConsoleService::info("PubSub Message");

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!("Sender => {}", from));

        if from != live.streamer_peer_id {
            #[cfg(debug_assertions)]
            ConsoleService::warn("Unauthorized Sender");
            return;
        }

        let cid = match Cid::try_from(data) {
            Ok(cid) => cid,
            Err(e) => {
                #[cfg(debug_assertions)]
                ConsoleService::error(&format!("{:?}", e));
                return;
            }
        };

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!("Message => {}", cid));

        let cb = self.link.callback(Msg::VideoNode);
        spawn_local(ipfs_dag_get_callback(cid, cb));

        if self.media_buffers.is_none() {
            let cb = self.link.callback_once(Msg::SetupNode);

            spawn_local(ipfs_dag_get_path_callback::<_, TempSetupNode, SetupNode>(
                cid, "/setup/", cb,
            ));

            return;
        }
    }

    /// Callback when source buffer is done updating.
    fn on_update_end(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("On Update End");

        self.tick()
    }

    /// Callback when video element has seeked.
    fn on_seeking(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("On Seeking");

        self.state = MachineState::Flush;
    }

    /// Has waited 1 second, update state machine now.
    fn on_timeout(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("On Timeout");

        self.timeout_closure = None;
        self.handle = 0;

        self.tick()
    }

    /// Update state machine.
    fn tick(&mut self) {
        match self.state {
            MachineState::Load => self.load_segment(),
            MachineState::Switch => self.switch_quality(),
            MachineState::Flush => self.flush_buffer(),
            MachineState::Timeout => self.set_timeout(),
            MachineState::Status => self.check_status(),
            MachineState::AdaptativeBitrate => self.check_abr(),
        }
    }

    /// Set 1 second timeout.
    fn set_timeout(&mut self) {
        if self.timeout_closure.is_some() {
            return;
        }

        let cb = self.link.callback(|_| Msg::Timeout);

        let closure = Closure::wrap(Box::new(move || cb.emit(())) as Box<dyn Fn()>);

        match self
            .window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                1000,
            ) {
            Ok(handle) => self.handle = handle,
            Err(e) => ConsoleService::error(&format!("{:?}", e)),
        }

        self.timeout_closure = Some(closure);
    }

    fn buffer_video_node(&mut self, cid: Cid, node: VideoNode) {
        let live = self.live_stream.unwrap();

        if live.buffer.is_empty() && node.previous.map(|l| l.link) == live.previous {
            live.buffer.push_back((cid, node));
            return self.check_order();
        } else if node.previous.map(|l| l.link) == live.buffer.back().map(|(cid, _)| *cid) {
            live.buffer.push_back((cid, node));
            return self.check_order();
        }

        #[cfg(debug_assertions)]
        ConsoleService::info("Out of Order Node");

        live.unordered_buffer.insert(cid, node);

        let cid = node.previous.map(|l| l.link).unwrap();
        let cb = self.link.callback(Msg::VideoNode);

        spawn_local(ipfs_dag_get_callback(cid, cb));
    }

    /// Check recursively if unordered nodes match buffer order.
    fn check_order(&mut self) {
        let live = self.live_stream.unwrap();

        let cid = match live.buffer.back().map(|(cid, _)| cid) {
            Some(cid) => cid,
            None => return,
        };

        if let Some(node) = live.unordered_buffer.remove(cid) {
            #[cfg(debug_assertions)]
            ConsoleService::info("Node Reordered");

            live.buffer.push_back((*cid, node));

            return self.check_order();
        }
    }

    /// Create source buffer then load initialization segment.
    fn add_source_buffer(&mut self, setup_node: SetupNode) {
        #[cfg(debug_assertions)]
        ConsoleService::info("Adding Source Buffer");

        if self.media_source.ready_state() != MediaSourceReadyState::Open {
            #[cfg(debug_assertions)]
            ConsoleService::info("Media Source Not Open");
            return;
        }

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!(
            "Setup Node \n {}",
            &serde_json::to_string_pretty(&setup_node).expect("Can't print")
        ));

        #[cfg(debug_assertions)]
        ConsoleService::info("Listing Tracks");

        let mut audio_buffer = None;
        let mut video_buffer = None;

        for (level, track) in setup_node.tracks.iter().enumerate() {
            if !MediaSource::is_type_supported(&track.codec) {
                ConsoleService::error(&format!("MIME Type {:?} unsupported", &track.codec));
                continue;
            }

            #[cfg(debug_assertions)]
            ConsoleService::info(&format!(
                "Level {} Name {} Codec {} Bandwidth {}",
                level, track.name, track.codec, track.bandwidth
            ));

            if video_buffer.is_some() {
                continue;
            }

            if track.name == "audio" && audio_buffer.is_some() {
                continue;
            }

            let source_buffer = match self.media_source.add_source_buffer(&track.codec) {
                Ok(sb) => sb,
                Err(e) => {
                    ConsoleService::error(&format!("{:?}", e));
                    return;
                }
            };

            if track.name == "audio" {
                audio_buffer = Some(source_buffer);
            } else {
                video_buffer = Some(source_buffer);
            }
        }

        let media_buffer = MediaBuffers {
            audio: audio_buffer.unwrap(),
            video: video_buffer.unwrap(),
            tracks: setup_node.tracks,
        };

        let cb = self.link.callback(|_| Msg::UpdateEnd);
        let closure = Closure::wrap(Box::new(move || cb.emit(())) as Box<dyn Fn()>);
        media_buffer
            .video
            .set_onupdateend(Some(closure.as_ref().unchecked_ref()));

        self.update_end_closure = Some(closure);

        let audio_path = media_buffer.tracks[0]
            .initialization_segment
            .link
            .to_string();
        let video_path = media_buffer.tracks[1]
            .initialization_segment
            .link
            .to_string();

        self.media_buffers = Some(media_buffer);
        self.state = MachineState::Load;

        let cb = self.link.callback(Msg::Append);

        spawn_local(audio_video_cat(audio_path, video_path, cb));
    }

    /// Load either live or VOD segment.
    fn load_segment(&self) {
        if self.metadata.is_some() {
            return self.load_media_segment();
        }

        self.load_live_segment();
    }

    /// Try get cid from live buffer then fetch video data from ipfs.
    fn load_live_segment(&mut self) {
        let live = self.live_stream.unwrap();

        let (cid, node) = match live.buffer.pop_front() {
            Some(cid) => cid,
            None => return self.set_timeout(),
        };

        #[cfg(debug_assertions)]
        ConsoleService::info("Loading Live Media Segments");

        live.previous = Some(cid);

        let track_name = self.media_buffers.unwrap().tracks[self.level].name;

        let audio_path = node.tracks["audio"].link.to_string();
        let video_path = node.tracks[&track_name].link.to_string();

        let cb = self.link.callback(Msg::Append);

        self.state = MachineState::AdaptativeBitrate;
        self.ema.start_timer();

        spawn_local(audio_video_cat(audio_path, video_path, cb));
    }

    /// Get CID from timecode then fetch video data from ipfs.
    fn load_media_segment(&self) {
        let metadata = self.metadata.unwrap();
        let buffers = self.media_buffers.unwrap();

        let track_name = &buffers.tracks[self.level].name;

        let time_ranges = match buffers.video.buffered() {
            Ok(tm) => tm,
            Err(_) => {
                #[cfg(debug_assertions)]
                ConsoleService::info("Buffer empty");
                return;
            }
        };

        let mut buff_end = 0.0;

        let count = time_ranges.length();

        if count > 0 {
            if let Ok(end) = time_ranges.end(count - 1) {
                buff_end = end;
            }
        }

        //if buffer is empty load at current time
        if buff_end <= 0.0 {
            let current_time = match self.media_element.as_ref() {
                Some(media_element) => media_element.current_time(),
                None => {
                    #[cfg(debug_assertions)]
                    ConsoleService::info("No Media Element");
                    return;
                }
            };

            if current_time > 1.0 {
                buff_end = current_time - 1.0;
            }
        }

        let (hours, minutes, seconds) = seconds_to_timecode(buff_end);

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!(
            "Loading Media Segments at timecode {}:{}:{}",
            hours, minutes, seconds
        ));

        let audio_path = format!(
            "{}/time/hour/{}/minute/{}/second/{}/video/track/audio",
            metadata.video.link.to_string(),
            hours,
            minutes,
            seconds,
        );

        let video_path = format!(
            "{}/time/hour/{}/minute/{}/second/{}/video/track/{}",
            metadata.video.link.to_string(),
            hours,
            minutes,
            seconds,
            track_name,
        );

        let cb = self.link.callback(Msg::Append);

        self.state = MachineState::AdaptativeBitrate;
        self.ema.start_timer();

        spawn_local(audio_video_cat(audio_path, video_path, cb));
    }

    /// Recalculate download speed then set quality level.
    fn check_abr(&mut self) {
        let buffers = self.media_buffers.unwrap();

        let bandwidth = buffers.tracks[self.level].bandwidth as f64;

        let avg_bitrate = match self.ema.recalculate_average_speed(bandwidth) {
            Some(at) => at,
            None => {
                self.state = MachineState::Status;
                return self.tick();
            }
        };

        let mut next_level = 1; // start at 1 since 0 is audio
        while let Some(next_bitrate) = buffers.tracks.get(next_level + 1).map(|t| t.bandwidth) {
            if avg_bitrate <= next_bitrate as f64 {
                break;
            }

            next_level += 1;
        }

        if next_level == self.level {
            self.state = MachineState::Status;
            return self.tick();
        }

        self.level = next_level;
        self.state = MachineState::Switch;
        self.tick()
    }

    /// Check buffers and current time then trigger new action.
    fn check_status(&mut self) {
        let buffers = self.media_buffers.unwrap();

        let time_ranges = match buffers.video.buffered() {
            Ok(tm) => tm,
            Err(_) => {
                #[cfg(debug_assertions)]
                ConsoleService::info("Buffer empty");
                return self.set_timeout();
            }
        };

        let count = time_ranges.length();

        let mut buff_start = 0.0;
        let mut buff_end = 0.0;

        for i in 0..count {
            if let Ok(start) = time_ranges.start(i) {
                buff_start = start;
            }

            if let Ok(end) = time_ranges.end(i) {
                buff_end = end;
            }

            #[cfg(debug_assertions)]
            ConsoleService::info(&format!(
                "Time Range {} buffers {}s to {}s",
                i, buff_start, buff_end
            ));
        }

        let current_time = match self.media_element.as_ref() {
            Some(media_element) => media_element.current_time(),
            None => {
                #[cfg(debug_assertions)]
                ConsoleService::info("No Media Element");
                return self.set_timeout();
            }
        };

        if current_time > buff_start + BACK_BUFFER_LENGTH {
            #[cfg(debug_assertions)]
            ConsoleService::info("Back Buffer Full");
            return self.flush_buffer();
        }

        if self.metadata.is_some() && buff_end >= self.metadata.unwrap().duration {
            #[cfg(debug_assertions)]
            ConsoleService::info("End Of Video");
            return;
        }

        if self.metadata.is_some() && current_time + FORWARD_BUFFER_LENGTH < buff_end {
            #[cfg(debug_assertions)]
            ConsoleService::info("Forward Buffer Full");
            return self.set_timeout();
        }

        self.load_segment()
    }

    /// Flush everything or just back buffer.
    fn flush_buffer(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("Flushing Buffer");

        let buffers = self.media_buffers.unwrap();

        let time_ranges = match buffers.video.buffered() {
            Ok(tm) => tm,
            Err(_) => {
                #[cfg(debug_assertions)]
                ConsoleService::info("Buffer empty");
                return;
            }
        };

        let count = time_ranges.length();

        let mut buff_start = 0.0;
        let mut buff_end = 0.0;

        for i in 0..count {
            if let Ok(start) = time_ranges.start(i) {
                buff_start = start;
            }

            if let Ok(end) = time_ranges.end(i) {
                buff_end = end;
            }
        }

        let current_time = match self.media_element.as_ref() {
            Some(media_element) => media_element.current_time(),
            None => {
                #[cfg(debug_assertions)]
                ConsoleService::info("No Media Element");
                return;
            }
        };

        let back_buffer_start = current_time - BACK_BUFFER_LENGTH;

        //full flush except if back buffer flush is possible
        if buff_start < back_buffer_start {
            buff_end = back_buffer_start
        }

        if let Err(e) = buffers.audio.remove(buff_start, buff_end) {
            ConsoleService::error(&format!("{:?}", e));
            return;
        }

        if let Err(e) = buffers.video.remove(buff_start, buff_end) {
            ConsoleService::error(&format!("{:?}", e));
            return;
        }

        self.state = MachineState::Load;
    }

    /// Switch source buffer codec then load initialization segment.
    fn switch_quality(&mut self) {
        #[cfg(debug_assertions)]
        ConsoleService::info("Switching Quality");

        let buffers = self.media_buffers.unwrap();

        let track = match buffers.tracks.get(self.level) {
            Some(track) => track,
            None => return,
        };

        if let Err(e) = buffers.video.change_type(&track.codec) {
            ConsoleService::error(&format!("{:?}", e));
            return;
        }

        #[cfg(debug_assertions)]
        ConsoleService::info(&format!(
            "Level {} Name {} Codec {} Bandwidth {}",
            self.level, track.name, track.codec, track.bandwidth
        ));

        let cid = track.initialization_segment.link;

        self.state = MachineState::Load;

        let cb = self.link.callback(Msg::Append);

        spawn_local(init_cat(cid, cb));
    }

    /// Append audio and video segments to the buffers.
    fn append_buffers(&self, audio_res: Option<Uint8Array>, vid_seg: Uint8Array) {
        let buffers = self.media_buffers.as_ref().unwrap();

        if let Some(aud_seg) = audio_res {
            if let Err(e) = buffers.audio.append_buffer_with_array_buffer_view(&aud_seg) {
                ConsoleService::warn(&format!("{:#?}", e));
            }
        }

        if let Err(e) = buffers.video.append_buffer_with_array_buffer_view(&vid_seg) {
            ConsoleService::warn(&format!("{:#?}", e));
        }
    }
}

/// Translate total number of seconds to timecode.
pub fn seconds_to_timecode(seconds: f64) -> (u8, u8, u8) {
    let rem_seconds = seconds.round();

    let hours = (rem_seconds / 3600.0) as u8;
    let rem_seconds = rem_seconds.rem_euclid(3600.0);

    let minutes = (rem_seconds / 60.0) as u8;
    let rem_seconds = rem_seconds.rem_euclid(60.0);

    let seconds = rem_seconds as u8;

    (hours, minutes, seconds)
}
