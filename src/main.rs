use clap::Parser;
use crossbeam::queue::ArrayQueue;
use device_query::{DeviceQuery, DeviceState, Keycode};
use rdev::{Button, EventType, listen};
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
    LControlW,
    LControlA,
    LControlS,
    LControlD,
    LControlQ,
    LControlE,
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

    #[clap(short, long, default_value = "5")]
    /// Retry time when call api failed
    retry: u8,
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
        if let Err(error) = listen(move |event| {
            let pressed = match event.event_type {
                EventType::ButtonPress(button) => MouseButton::from(button),
                _ => MouseButton::None,
            };
            let event = Event::from(pressed);
            if event != Event::Silent {
                mouse_tx.force_push(event);
            }
        }) {
            tracing::error!("Error listening for mouse events: {:?}", error);
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
                let event = Event::from(key_combination);
                if event != Event::Silent {
                    keyboard_tx.force_push(event);
                    last_pressed = key_combination;
                }
            }
            thread::sleep(SLEEP_DURATION);
        }
    });

    thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap()
            .handle()
            .block_on(async move {
                let _ = tokio::join!(
                    spawn(&args.url, mouse_queue, "Mouse Logger", args.retry),
                    spawn(&args.url, keyboard_queue, "Keyboard Logger", args.retry),
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

impl From<Button> for MouseButton {
    fn from(button: Button) -> MouseButton {
        match button {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
            Button::Unknown(id) => {
                #[cfg(not(unix))]
                if id == 2 {
                    MouseButton::Side1
                } else if id == 1 {
                    MouseButton::Side2
                } else {
                    MouseButton::None
                }
                #[cfg(unix)]
                if id == 9 {
                    MouseButton::Side1
                } else if id == 8 {
                    MouseButton::Side2
                } else {
                    MouseButton::None
                }
            }
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
        if keys.contains(&Keycode::LControl) && keys.len() == 2 {
            if keys.contains(&Keycode::W) {
                KeyCombination::LControlW
            } else if keys.contains(&Keycode::A) {
                KeyCombination::LControlA
            } else if keys.contains(&Keycode::S) {
                KeyCombination::LControlS
            } else if keys.contains(&Keycode::D) {
                KeyCombination::LControlD
            } else if keys.contains(&Keycode::Q) {
                KeyCombination::LControlQ
            } else if keys.contains(&Keycode::E) {
                KeyCombination::LControlE
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
            KeyCombination::LControlW => Event::AimModeHead,
            KeyCombination::LControlA => Event::AimModeNeck,
            KeyCombination::LControlS => Event::AimModeChest,
            KeyCombination::LControlD => Event::AimModeAbdomen,
            KeyCombination::LControlQ => Event::AimOff,
            KeyCombination::LControlE => Event::AimOn,
            _ => Event::Silent,
        }
    }
}

fn spawn(
    url: &str,
    queue: Arc<ArrayQueue<Event>>,
    task_name: &str,
    retry: u8,
) -> tokio::task::JoinHandle<()> {
    let url = url.to_string();
    let task_name = task_name.to_string();
    tokio::spawn(async move {
        tracing::info!("Spawning task {}...", task_name);
        let client = reqwest::Client::new();
        loop {
            if let Some(event) = queue.pop() {
                tracing::info!("Received event: {:?}", event);
                if event != Event::Silent {
                    let id = event as usize;
                    for _ in 0..retry {
                        let result = client
                            .put(format!("{url}/stream/event/{id}"))
                            .timeout(Duration::from_secs(2))
                            .send()
                            .await;
                        match result {
                            Ok(resp) => {
                                if resp.status().is_success() {
                                    tracing::info!("Sent event {event:?} successfully",);
                                    break;
                                } else {
                                    tracing::error!(
                                        "Sent event {event:?} failed with status {:?}",
                                        resp.status()
                                    );
                                }
                            }
                            Err(err) => tracing::error!("{:?}", err),
                        }
                    }
                }
            }
            tokio::time::sleep(SLEEP_DURATION).await;
        }
    })
}
