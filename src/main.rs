use clap::Parser;
use crossbeam::queue::ArrayQueue;
use device_query::{DeviceQuery, DeviceState, Keycode};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const SLEEP_DURATION: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, PartialEq, Copy)]
enum MouseButton {
    Left,
    Right,
    Middle,
    Side1,
    Side2,
    None,
}

#[derive(Clone, Debug, Copy, PartialEq)]
enum KeyCombination {
    LAlt1,
    LAlt2,
    LAlt3,
    LAlt4,
    None,
}

#[derive(Clone, Debug, Copy, PartialEq)]
enum Event {
    AimOff = 0,
    AimOn = 1,
    AimModeHead = 2,
    AimModeNeck = 3,
    AimModeChest = 4,
    AimModeAbdomen = 5,
    Silent = 6,
}

#[derive(Parser)]
struct Args {
    #[clap(short, long, default_value = "http://localhost:10000")]
    /// The URL of the Aimbot's event listener is serving
    url: String,
}

fn main() {
    tracing_subscriber::fmt::init();
    let mut args = Args::parse();
    args.url = args.url.trim_end_matches('/').to_string();
    let mouse_queue = Arc::new(ArrayQueue::new(1));
    let keyboard_queue = Arc::new(ArrayQueue::new(1));

    let mouse_tx = mouse_queue.clone();
    thread::spawn(move || {
        tracing::info!("Waiting for mouse input...");
        let device_state = DeviceState::new();
        let mut last_pressed = MouseButton::None;
        loop {
            let mouse = device_state.get_mouse();
            mouse
                .button_pressed
                .into_iter()
                .enumerate()
                .for_each(|(index, pressed)| {
                    if pressed {
                        let pressed = MouseButton::from(index);
                        if pressed != last_pressed {
                            mouse_tx.force_push(Event::from(pressed));
                            last_pressed = pressed;
                        }
                    }
                });
            thread::sleep(SLEEP_DURATION);
        }
    });

    let keyboard_tx = keyboard_queue.clone();
    thread::spawn(move || {
        tracing::info!("Waiting for keyboard input...");
        let device_state = DeviceState::new();
        let mut last_pressed = KeyCombination::None;
        loop {
            let keys_pressed = device_state.get_keys();
            let key_combination = KeyCombination::from(keys_pressed);
            if key_combination != last_pressed {
                keyboard_tx.force_push(Event::from(key_combination));
                last_pressed = key_combination;
            }
            thread::sleep(SLEEP_DURATION);
        }
    });

    thread::spawn(move || {
        tracing::info!("Starting event consumer...");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let _ = tokio::join!(
                    spawn(&args.url, mouse_queue),
                    spawn(&args.url, keyboard_queue)
                );
            });
    });

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .unwrap();
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1000));
    }
}

impl From<usize> for MouseButton {
    fn from(number: usize) -> MouseButton {
        match number {
            1 => MouseButton::Left,
            2 => MouseButton::Right,
            3 => MouseButton::Middle,
            4 => MouseButton::Side1,
            5 => MouseButton::Side2,
            _ => MouseButton::None,
        }
    }
}

impl From<MouseButton> for Event {
    fn from(button: MouseButton) -> Event {
        match button {
            MouseButton::Side1 => Event::AimOff,
            MouseButton::Side2 => Event::AimOn,
            _ => Event::Silent,
        }
    }
}

impl From<Vec<Keycode>> for KeyCombination {
    fn from(keys: Vec<Keycode>) -> KeyCombination {
        if keys.contains(&Keycode::LAlt) && keys.len() == 2 {
            if keys.contains(&Keycode::Numpad1) {
                KeyCombination::LAlt1
            } else if keys.contains(&Keycode::Numpad2) {
                KeyCombination::LAlt2
            } else if keys.contains(&Keycode::Numpad3) {
                KeyCombination::LAlt3
            } else if keys.contains(&Keycode::Numpad4) {
                KeyCombination::LAlt4
            } else {
                KeyCombination::None
            }
        } else {
            KeyCombination::None
        }
    }
}

impl From<KeyCombination> for Event {
    fn from(comb: KeyCombination) -> Event {
        match comb {
            KeyCombination::LAlt1 => Event::AimModeHead,
            KeyCombination::LAlt2 => Event::AimModeNeck,
            KeyCombination::LAlt3 => Event::AimModeChest,
            KeyCombination::LAlt4 => Event::AimModeAbdomen,
            _ => Event::Silent,
        }
    }
}

fn spawn(url: &str, queue: Arc<ArrayQueue<Event>>) -> tokio::task::JoinHandle<()> {
    let url = url.to_string();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        loop {
            if let Some(event) = queue.pop() {
                if event != Event::Silent {
                    let id = event as usize;
                    let result = client
                        .put(format!("{url}/stream/event/{id}"))
                        .timeout(Duration::from_secs(10))
                        .send()
                        .await;
                    match result {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                tracing::info!("Sent event {event:?} successfully",);
                            }
                        }
                        Err(err) => tracing::error!("{:?}", err),
                    }
                }
            }
        }
    })
}
