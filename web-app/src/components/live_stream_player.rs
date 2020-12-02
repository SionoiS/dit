use crate::agents::load_live_stream;
use crate::bindings;

use yew::prelude::{html, Component, ComponentLink, Html, Properties, ShouldRender};

#[derive(Clone, Debug, Eq, PartialEq, Properties)]
pub struct LiveStreamPlayer {
    pub topic: String,
}

impl Component for LiveStreamPlayer {
    type Message = ();
    type Properties = Self;

    fn create(props: Self::Properties, _link: ComponentLink<Self>) -> Self {
        props
    }

    fn update(&mut self, _msg: Self::Message) -> ShouldRender {
        false
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            <video id="video" autoplay=true controls=true muted=true poster="../live_like_poster.png" />
        }
    }

    fn rendered(&mut self, first_render: bool) {
        if first_render {
            load_live_stream(self.topic.clone());
        }
    }

    fn destroy(&mut self) {
        bindings::ipfs_unsubscribe(self.topic.clone().into());
    }
}
