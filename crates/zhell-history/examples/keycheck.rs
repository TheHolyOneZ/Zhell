fn main() {
    let path = std::env::args().nth(1).expect("path");
    let key = zhell_history::history_key().expect("keyring");
    if std::env::args().nth(2).as_deref() == Some("decrypt") {
        zhell_history::decrypt_in_place(std::path::Path::new(&path), &key).expect("decrypt");
        println!("decrypted");
        return;
    }
    let db = zhell_history::History::open_with_key(std::path::Path::new(&path), Some(&key)).expect("open");
    drop(db);
    println!("ok, key len {}", key.len());
    let again = zhell_history::history_key().unwrap();
    println!("stable: {}", again == key);
}
