#![no_std]

#[derive(serde::Deserialize)]
pub struct UpdateResponse<'a> {
    #[serde(borrow)]
    pub result: heapless::Vec<Update<'a>, 8>, // max 8 updates buffered
}

#[derive(serde::Deserialize)]
pub struct Update<'a> {
    pub update_id: i64,
    #[serde(borrow)]
    pub message: Option<Message<'a>>,
}

#[derive(serde::Deserialize)]
pub struct Message<'a> {
    pub text: Option<&'a str>,
    pub chat: Chat,
}

#[derive(serde::Deserialize)]
pub struct Chat {
    pub id: i64,
}
