CREATE TABLE IF NOT EXISTS metering_points (
    id text PRIMARY KEY CHECK (id ~ '^[0-9]{18}$'),
    type_of_mp text NOT NULL,
    street_name text NOT NULL,
    building_number text NOT NULL,
    floor text,
    door text,
    postcode text NOT NULL,
    city text,
    direction text NOT NULL CHECK (direction IN ('consumption', 'production', 'grid-export')),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS usage_periods (
    metering_point_id text NOT NULL REFERENCES metering_points(id),
    direction text NOT NULL CHECK (direction IN ('consumption', 'production', 'grid-export')),
    granularity text NOT NULL CHECK (granularity IN ('day', 'week', 'month', 'year')),
    period_start date NOT NULL,
    period_end date NOT NULL CHECK (period_end > period_start),
    milli_kwh bigint CHECK (milli_kwh >= 0),
    kwh numeric GENERATED ALWAYS AS (milli_kwh::numeric / 1000) STORED,
    quality text NOT NULL CHECK (quality IN ('A01', 'A02', 'A03', 'A04', 'A05')),
    complete boolean NOT NULL,
    fetched_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (metering_point_id, direction, granularity, period_start),
    CHECK (NOT complete OR milli_kwh IS NOT NULL)
);

CREATE OR REPLACE VIEW daily_usage AS
    SELECT * FROM usage_periods WHERE granularity = 'day';
CREATE OR REPLACE VIEW weekly_usage AS
    SELECT * FROM usage_periods WHERE granularity = 'week';
CREATE OR REPLACE VIEW monthly_usage AS
    SELECT * FROM usage_periods WHERE granularity = 'month';
CREATE OR REPLACE VIEW yearly_usage AS
    SELECT * FROM usage_periods WHERE granularity = 'year';
