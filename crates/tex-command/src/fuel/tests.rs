use super::*;

#[test]
fn exact_limit_funds_exactly_that_many_actions() {
    let mut fuel = CommandFuel::new(3).expect("valid test limit");
    assert!(fuel.charge().is_ok());
    assert!(fuel.charge().is_ok());
    assert!(fuel.charge().is_ok());
    assert_eq!(
        fuel.charge(),
        Err(crate::CommandError::FuelExhausted {
            limit: 3,
            burned: 3,
            work: CommandWorkCounters {
                fuel_charges: 3,
                ..CommandWorkCounters::default()
            },
        })
    );
    assert_eq!(fuel.burned(), 3);
}

#[test]
fn authority_scale_limits_publish_exact_terminal_counts() {
    for limit in [1_000_000, 50_000_000] {
        let mut fuel = CommandFuel::new(limit).expect("valid authority limit");
        for expected_burned in 1..=limit {
            fuel.charge().expect("limit funds the exact charge");
            if expected_burned == 1 || expected_burned == limit {
                assert_eq!(fuel.burned(), expected_burned);
            }
        }
        assert_eq!(
            fuel.charge(),
            Err(crate::CommandError::FuelExhausted {
                limit,
                burned: limit,
                work: CommandWorkCounters {
                    fuel_charges: limit,
                    ..CommandWorkCounters::default()
                },
            })
        );
    }
}

#[test]
#[cfg(feature = "profiling")]
fn published_work_derives_fuel_without_mutating_detail_counters() {
    let mut fuel = CommandFuel::new(2).expect("valid test limit");
    fuel.record_raw_delivery(true, true, RawDeliveryKind::StoredToken);
    fuel.record_expanded_delivery();
    fuel.record_write_expansion();
    fuel.charge().expect("first charge");

    assert_eq!(
        fuel.work(),
        CommandWorkCounters {
            fuel_charges: 1,
            token_frame_steps: 1,
            expanded_deliveries: 1,
            meaning_lookups: 1,
            scanner_tokens: 1,
            write_expansions: 1,
            raw_delivery_kinds: [0, 1, 0, 0],
        }
    );
    assert_eq!(
        fuel.work().raw_delivery_kinds.into_iter().sum::<u64>(),
        fuel.work().token_frame_steps,
    );
    assert_eq!(fuel.remaining, 1);
}

#[test]
#[cfg(not(feature = "profiling"))]
fn production_ledger_stores_only_the_runaway_guard() {
    assert_eq!(
        std::mem::size_of::<CommandFuel>(),
        2 * std::mem::size_of::<u64>()
    );

    let mut fuel = CommandFuel::new(2).expect("valid test limit");
    fuel.charge().expect("first charge");
    assert_eq!(
        fuel.work(),
        CommandWorkCounters {
            fuel_charges: 1,
            ..CommandWorkCounters::default()
        }
    );
}

#[test]
fn invalid_limits_are_rejected_instead_of_becoming_unlimited() {
    assert_eq!(MAX_COMMAND_FUEL_LIMIT, 100_000_000_000);
    for requested in [0, MAX_COMMAND_FUEL_LIMIT + 1, u64::MAX] {
        let error = CommandFuel::new(requested).expect_err("invalid limit");
        assert_eq!(error, CommandFuelLimitError { requested });
        assert_eq!(
            error.to_string(),
            format!(
                "canonical command fuel limit {requested} is outside 1..={MAX_COMMAND_FUEL_LIMIT}"
            )
        );
    }
    assert_eq!(CommandFuel::new(1).expect("minimum").limit(), 1);
    assert_eq!(
        CommandFuel::new(MAX_COMMAND_FUEL_LIMIT)
            .expect("maximum")
            .limit(),
        MAX_COMMAND_FUEL_LIMIT
    );
}

#[test]
fn default_is_positive_finite_and_within_the_hard_maximum() {
    let fuel = CommandFuelLedger::default();
    assert_eq!(fuel.limit(), DEFAULT_COMMAND_FUEL_LIMIT);
    assert!(fuel.limit() > 0);
    assert!(fuel.limit() <= MAX_COMMAND_FUEL_LIMIT);
    assert_ne!(fuel.limit(), u64::MAX);
}
