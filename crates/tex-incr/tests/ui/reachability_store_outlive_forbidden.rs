use tex_incr::{RevisionId, Session, new_reachability_store};

fn escaped_session() -> Session<'static> {
    let store = new_reachability_store();
    Session::start(&store, "escape", RevisionId::new(1), "\\end", 0).unwrap()
}

fn main() {
    let _ = escaped_session();
}
