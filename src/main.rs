mod openmeteo;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use alfred_core::{tokio, AlfredModule, ModuleDetailsBuilder};
use alfred_core::log::{debug, warn};
use alfred_core::message::MessageType;
use open_meteo_rs::{Client, Location};
use open_meteo_rs::forecast::{ForecastResult, ForecastResultItem, Options as ForecastOptions};
use open_meteo_rs::geocoding::Options as GeocodingOptions;
use crate::openmeteo::wmo_to_str;

const MODULE_NAME: &str = "openmeteo";
const INPUT_TOPIC: &str = "weather";
const ACTUAL_WEATHER_TOPIC: &str = "weather.actual";
const FORECAST_WEATHER_TOPIC: &str = "weather.forecast";

async fn get_location(client: &Client, city_name: &str) -> Location {
    let mut options = GeocodingOptions::default();
    options.name = Some(city_name.into());
    options.count = Some(1);
    options.language = Some("it".into());
    client.geocoding(options).await.unwrap()
        .results.unwrap().first()
        .map(|geo| Location {
            lat: geo.latitude.unwrap(),
            lng: geo.longitude.unwrap()
        })
        .unwrap()
}

async fn get_forecast(client: &Client, city_name: &str) -> Result<ForecastResult, Box<dyn Error>> {
    let location = get_location(&client, city_name).await;
    let mut forecast_options = ForecastOptions::default();
    forecast_options.location = location;
    forecast_options.temperature_unit = Some(open_meteo_rs::forecast::TemperatureUnit::Celsius);
    forecast_options.wind_speed_unit = Some(open_meteo_rs::forecast::WindSpeedUnit::Kmh);
    forecast_options.precipitation_unit = Some(open_meteo_rs::forecast::PrecipitationUnit::Millimeters);
    forecast_options.daily.push("temperature_2m".into());
    forecast_options.daily.push("weather_code".into());
    forecast_options.current.push("temperature_2m".into());
    forecast_options.current.push("weather_code".into());
    client.forecast(forecast_options).await
}

fn forecast_result_items_to_string(values: HashMap<String, ForecastResultItem>) -> String {
    let temperature = values.get("temperature_2m")
        .ok_or(Box::new("No temperature values found")).unwrap();
    let wmo_code = values.get("weather_code")
        .ok_or(Box::new("No weather code values found")).unwrap()
        .value.clone().to_string();
    format!("{} {}; {}", temperature.value, temperature.unit.clone().unwrap(), wmo_to_str(wmo_code).unwrap_or("Unknown"))
}

async fn get_actual_weather(client: &Client, city_name: &str) -> Result<String, Box<dyn Error>> {
    get_forecast(client, city_name).await
        .map(|result| {
            let current_result = result.current
                .ok_or(Box::new("No current value found")).unwrap();
            forecast_result_items_to_string(current_result.values)
        })
}

async fn get_forecast_weather(client: &Client, city_name: &str) -> Result<String, Box<dyn Error>> {
    get_forecast(client, city_name).await
        .map(|result| {
            let forecasts = result.daily
                .ok_or(Box::new("No current value found")).unwrap();
            let today = forecast_result_items_to_string(forecasts.first().unwrap().values.clone());
            let tomorrow = forecast_result_items_to_string(forecasts.get(1).unwrap().values.clone());
            format!("Today: {}.\nTomorrow {}", today, tomorrow)
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let open_meteo_client = Client::new();
    let res = get_actual_weather(&open_meteo_client, "Brescia").await?;
    println!("{}", res);
    let module_details = ModuleDetailsBuilder::new()
        .module_name(MODULE_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .capabilities(BTreeMap::from([
            ("current weather of a city".to_string(), format!("{ACTUAL_WEATHER_TOPIC}: $city$")),
            ("forecast weather of a city (today and tomorrow)".to_string(), format!("{FORECAST_WEATHER_TOPIC}: $city$")),
        ]))
        .build();
    let mut module = AlfredModule::new_with_details(module_details).await?;
    module.listen(INPUT_TOPIC).await?;

    loop {
        let (topic, message) = module.receive().await?;
        let response_text = match topic.as_str() {
            ACTUAL_WEATHER_TOPIC => {
                get_actual_weather(&open_meteo_client, message.text.as_str()).await
            },
            FORECAST_WEATHER_TOPIC => {
                get_forecast_weather(&open_meteo_client, message.text.as_str()).await
            },
            _ => {
                warn!("Received unknown topic: {}", topic);
                Err("Unknown topic".into())
            }
        };
        if let Ok(response_text) = response_text {
            debug!("{response_text}");
            let (response_topic, response_message) = message.reply(response_text.to_string(), MessageType::Text)?;
            module.send(response_topic.as_str(), &response_message).await?;
        }
    }
}
