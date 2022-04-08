use std::sync::Arc;

use rand::{prelude::StdRng, Rng, SeedableRng};

const SCHEDULE_SEED: u64 = 13409387784011516370;

pub struct StdRngSchedule {
    rng: StdRng,
}

impl LeaderSchedule for StdRngSchedule {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            rng: StdRng::seed_from_u64(SCHEDULE_SEED),
        })
    }

    fn next(&mut self) -> [u8; 32] {
        self.rng.gen() // NOTE: weighted random done every epoch by a set of validators that we choose randomly based on the seed.
                       // the seed is somehow manipulated every epoch/block.
    }
}

pub trait LeaderSchedule {
    fn new() -> Arc<Self>
    where
        Self: Sized;

    fn next(&mut self) -> [u8; 32];
}

fn get_validator() -> [u8; 32] {
    // TODO: the db will have the list of the current validators (`stake` is a native contract)
    [0; 32]
}
