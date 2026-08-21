use super::{
    DetachedFormatImage, FormatError, FormatPublicationError, with_format_destination,
    with_materialized_format,
};
use crate::interner::InternerBudget;
use crate::world::{JobClock, World};
use crate::{InteractionMode, with_universe};

fn budget() -> InternerBudget {
    InternerBudget::new(32, 64, 4 * 1024).expect("test budget")
}

fn image() -> DetachedFormatImage {
    with_universe(budget(), |universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        universe.capture_format_image().expect("capture")
    })
    .expect("fresh universe")
}

#[test]
fn detached_image_roundtrips_bytes_and_rejects_corruption() {
    let image = image();
    let bytes = image.as_bytes().to_vec();

    assert_eq!(
        DetachedFormatImage::try_from_bytes(bytes.clone())
            .expect("validated bytes")
            .into_bytes(),
        bytes
    );

    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bad_magic).unwrap_err(),
        FormatError::BadMagic
    );
    let mut bad_checksum = bytes;
    let last = bad_checksum.len() - 1;
    bad_checksum[last] ^= 1;
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bad_checksum).unwrap_err(),
        FormatError::Checksum
    );
}

#[test]
fn one_borrowed_image_materializes_as_isolated_fresh_jobs() {
    let image = image();
    let first_clock = JobClock {
        time: 10,
        second: 20,
        day: 3,
        month: 4,
        year: 2027,
    };
    let second_clock = JobClock {
        time: 30,
        second: 40,
        day: 5,
        month: 6,
        year: 2028,
    };
    let first = with_materialized_format(
        budget(),
        World::memory_with_clock(first_clock),
        &image,
        |universe| {
            assert_eq!(universe.world().job_clock(), first_clock);
            assert_eq!(universe.interaction_mode(), InteractionMode::Nonstop);
            universe
                .capture_format_image()
                .expect("redump")
                .into_bytes()
        },
    )
    .expect("first load");
    let second = with_materialized_format(
        budget(),
        World::memory_with_clock(second_clock),
        &image,
        |universe| {
            assert_eq!(universe.world().job_clock(), second_clock);
            universe
                .capture_format_image()
                .expect("redump")
                .into_bytes()
        },
    )
    .expect("second load");

    assert_eq!(first, image.as_bytes());
    assert_eq!(second, image.as_bytes());
}

#[test]
fn foreign_staging_is_rejected_before_world_publication() {
    let image = image();
    with_format_destination(budget(), World::memory(), |destination| {
        let mut staging = destination.stage(&image)?;
        staging.destination = staging.destination.wrapping_add(1);
        assert_eq!(
            destination.materialize(staging, |_| ()),
            Err(FormatPublicationError::ForeignDestination)
        );
        assert!(destination.world.is_some());
        Ok(())
    })
    .expect("destination episode");
}

#[test]
fn staging_consumes_destination_once() {
    let image = image();
    with_format_destination(budget(), World::memory(), |destination| {
        let staging = destination.stage(&image)?;
        assert!(matches!(
            destination.stage(&image),
            Err(FormatError::DestinationConsumed)
        ));
        destination
            .materialize(staging, |_| ())
            .expect("matching publication");
        Ok(())
    })
    .expect("destination episode");
}
