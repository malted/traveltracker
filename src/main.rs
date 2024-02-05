#![feature(async_closure)]

use parking_lot::RwLock;
use status_bar::{ns_alert, sync_infinite_event_loop, Menu, MenuItem, StatusItem};
use std::sync::{
    atomic::{AtomicU8, Ordering::SeqCst},
    Arc,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: check if the wifi name is OEBB
    start_statusbar().await?;

    Ok(())
}

async fn start_statusbar() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    let (sender, receiver) = std::sync::mpsc::channel::<()>();

    let speed = Arc::new(RwLock::new(String::new()));
    let speed2 = speed.clone();

    let combinedInfo = Arc::new(RwLock::new(serde_json::Value::Null));
    let combinedInfo2 = combinedInfo.clone();

    tokio::spawn(async move {
        loop {
            *speed2.write() = client
                .get("http://192.168.32.1/api/speed")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();

            *combinedInfo2.write() = client
                .get("http://192.168.32.1/assets/modules/fis/combined.json")
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();

            sender.send(()).unwrap();

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    let status_item = std::cell::RefCell::new(StatusItem::new("", Menu::new(vec![])));

    sync_infinite_event_loop(receiver, move |_| {
        let ci = combinedInfo.read();
        let next_name = ci
            .get("nextStation")
            .expect("get next station val")
            .get("name")
            .expect("get next station name")
            .get("de")
            .expect("de name")
            .as_str()
            .expect("de name as str");
        let forecast_arrival = ci
            .get("nextStation")
            .expect("get next station val")
            .get("arrival")
            .expect("get arrival val")
            .get("forecast")
            .expect("get forecast val")
            .as_str()
            .expect("forecast val as str");
        let train_type = ci
            .get("trainType")
            .expect("get train type val")
            .as_str()
            .expect("train type val as str");
        let trip_number = ci
            .get("tripNumber")
            .expect("get trip number val")
            .as_str()
            .expect("trip number val as str");
        let destination_name = ci
            .get("destination")
            .expect("get destination val")
            .get("all")
            .expect("get destination name")
            .as_str()
            .expect("de name as str");

        status_item
            .borrow_mut()
            .set_title(format!("{} km/h", speed.read().to_string()));
        status_item.borrow_mut().set_menu(Menu::new(vec![
            MenuItem::new(
                format!("On {train_type} {trip_number} to {destination_name}"),
                None,
                None,
            ),
            MenuItem::new(
                format!("Next station: {next_name} at {forecast_arrival}"),
                None,
                None,
            ),
            MenuItem::new(
                format!("Go to dashboard"),
                Some(Box::new(|| {
                    webbrowser::open("http://192.168.32.1").unwrap()
                })),
                None,
            ),
        ]));
    });

    Ok(())
}
