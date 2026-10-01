pub struct Session {
    pub id: i64,
    token: std::rc::Rc<i64>,
}

impl Session {
    pub fn open(id: i64) -> Session {
        Session {
            id,
            token: std::rc::Rc::new(id),
        }
    }
}

pub async fn report(session: Session) -> i64 {
    *session.token
}
