use std::sync::Mutex;

lazy_static::lazy_static! {
    pub static ref ADAPTER_PATH:  std::sync::Arc<Mutex<String>> =
    std::sync::Arc::new(Mutex::new(String::from("/org/bluez/hci0")));
}

lazy_static::lazy_static! {
    pub static ref GATT_APPLICATION_PATH:  std::sync::Arc<Mutex<String>> =
    std::sync::Arc::new(Mutex::new(String::from("/org/bluez/gatt_application")));
}

lazy_static::lazy_static! {
    pub static ref ADVERTISEMENT_PATH:  std::sync::Arc<Mutex<String>> =
    std::sync::Arc::new(Mutex::new(String::from("/org/bluez/diagnify/adv/0")));
}

// Getter function for ADAPTER_PATH
pub fn get_adapter_path() -> String {
    let global_string = ADAPTER_PATH.lock().unwrap();
    global_string.clone() // Return a copy to avoid locking issues
}

// Setter function for ADAPTER_PATH
pub fn set_adapter_path(new_value: &str) {
    let mut global_string = ADAPTER_PATH.lock().unwrap();
    *global_string = new_value.to_string();
}

// Getter function for GATT_APPLICATION_PATH
pub fn get_gatt_application_path() -> String {
    let global_string = GATT_APPLICATION_PATH.lock().unwrap();
    global_string.clone() // Return a copy to avoid locking issues
}

// Setter function for GATT_APPLICATION_PATH
pub fn set_gatt_application_path(new_value: &str) {
    let mut global_string = GATT_APPLICATION_PATH.lock().unwrap();
    *global_string = new_value.to_string();
}

// Getter function for ADVERTISEMENT_PATH
pub fn get_advertisement_path() -> String {
    let global_string = GATT_APPLICATION_PATH.lock().unwrap();
    global_string.clone() // Return a copy to avoid locking issues
}

// Setter function for ADVERTISEMENT_PATH
pub fn set_advertisement_path(new_value: &str) {
    let mut global_string = GATT_APPLICATION_PATH.lock().unwrap();
    *global_string = new_value.to_string();
}