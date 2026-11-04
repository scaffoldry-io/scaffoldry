-- Process v2 phase 4: a committee's votes on a user task. One row per voter; a voter may change their vote until the
-- step closes. The decision itself is one ledger entry with the tally and every vote.
CREATE TABLE IF NOT EXISTS process_votes (
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    voter VARCHAR(255) NOT NULL,
    vote VARCHAR(8) NOT NULL,
    comment TEXT,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (instance_id, step_id, voter)
);
