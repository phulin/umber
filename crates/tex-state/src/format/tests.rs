use super::{FormatDestination, FormatPublicationError};

#[derive(Debug, Eq, PartialEq)]
struct NonCloneStage {
    names: Vec<String>,
    local_rows: Vec<u32>,
}

#[test]
fn malformed_stage_leaves_the_destination_unchanged() {
    let destination = FormatDestination::new();
    let live = vec!["existing".to_owned()];

    let staged = destination.stage::<NonCloneStage, _>(|| Err("bad local reference"));

    assert_eq!(staged.err(), Some("bad local reference"));
    assert_eq!(live, ["existing"]);
}

#[test]
fn staging_is_destination_local_and_cannot_publish_elsewhere() {
    let first = FormatDestination::new();
    let mut second = FormatDestination::new();
    let staged = first
        .stage::<_, ()>(|| {
            Ok(NonCloneStage {
                names: vec!["alpha".to_owned()],
                local_rows: vec![1],
            })
        })
        .expect("stage");
    let mut published = Vec::new();

    assert_eq!(
        second.publish(staged, |value| published.extend(value.names)),
        Err(FormatPublicationError::ForeignDestination)
    );
    assert!(published.is_empty());
}

#[test]
fn complete_staging_publishes_once_without_cloning_its_graph() {
    let mut destination = FormatDestination::new();
    let staged = destination
        .stage::<_, ()>(|| {
            Ok(NonCloneStage {
                names: vec!["alpha".to_owned(), "beta".to_owned()],
                local_rows: vec![1, 2],
            })
        })
        .expect("stage");
    let mut live = None;

    destination
        .publish(staged, |value| live = Some(value))
        .expect("matching destination");

    assert_eq!(
        live,
        Some(NonCloneStage {
            names: vec!["alpha".to_owned(), "beta".to_owned()],
            local_rows: vec![1, 2],
        })
    );
}
