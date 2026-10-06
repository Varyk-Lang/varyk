// A `varyk-http` written for another compiler: it has no `Request`.

pub struct App {
    port: u16,
}

pub struct Response {
    status: u16,
}

impl Response {
    pub fn empty() -> Response {
        Response { status: 204 }
    }

    pub fn status(&self) -> u16 {
        self.status
    }
}

impl App {
    pub fn port(&self) -> u16 {
        self.port
    }
}
