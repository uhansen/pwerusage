CREATE TABLE IF NOT EXISTS electricity_prices (
    zone text NOT NULL,
    datetime timestamptz NOT NULL,
    provider_name text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('history', 'latest', 'forecast')),
    raw_value double precision NOT NULL CHECK (raw_value > '-Infinity'::float8 AND raw_value < 'Infinity'::float8),
    raw_unit text NOT NULL CHECK (raw_unit IN ('EUR/MWh', 'EUR/kWh', 'DKK/MWh', 'DKK/kWh')),
    eur_dkk_rate double precision NOT NULL CHECK (eur_dkk_rate > 0 AND eur_dkk_rate < 'Infinity'::float8),
    spot_dkk_kwh double precision NOT NULL CHECK (spot_dkk_kwh > '-Infinity'::float8 AND spot_dkk_kwh < 'Infinity'::float8),
    provider_markup_per_kwh double precision NOT NULL CHECK (provider_markup_per_kwh > '-Infinity'::float8 AND provider_markup_per_kwh < 'Infinity'::float8),
    provider_markup_currency text NOT NULL CHECK (provider_markup_currency IN ('EUR', 'DKK')),
    markup_dkk_kwh double precision NOT NULL CHECK (markup_dkk_kwh > '-Infinity'::float8 AND markup_dkk_kwh < 'Infinity'::float8),
    include_vat boolean NOT NULL,
    total_dkk_kwh double precision NOT NULL CHECK (total_dkk_kwh > '-Infinity'::float8 AND total_dkk_kwh < 'Infinity'::float8),
    fetched_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (zone, datetime, provider_name)
);

CREATE OR REPLACE VIEW daily_prices AS
SELECT zone, provider_name,
    (datetime AT TIME ZONE 'Europe/Copenhagen')::date AS day,
    include_vat,
    avg(spot_dkk_kwh) AS average_spot_dkk_kwh,
    avg(total_dkk_kwh) AS average_total_dkk_kwh,
    min(total_dkk_kwh) AS minimum_total_dkk_kwh,
    max(total_dkk_kwh) AS maximum_total_dkk_kwh,
    count(*) AS available_hours,
    count(*) FILTER (WHERE kind = 'forecast') AS upcoming_hours
FROM electricity_prices
GROUP BY zone, provider_name, (datetime AT TIME ZONE 'Europe/Copenhagen')::date, include_vat;
