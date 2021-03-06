#![recursion_limit = "1024"]

mod app;
mod components;
mod pages;
mod utils;

fn main() {
    yew::start_app::<app::App>();
}
