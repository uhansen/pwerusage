\set ON_ERROR_STOP on

CREATE ROLE saveeye_owner NOLOGIN;
CREATE ROLE saveeye_listener LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
-- psql prompts without putting the password in SQL history or process arguments.
\password saveeye_listener
CREATE DATABASE saveeye OWNER saveeye_owner;
REVOKE ALL ON DATABASE saveeye FROM PUBLIC;
GRANT CONNECT ON DATABASE saveeye TO saveeye_listener;

\connect saveeye
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
SET ROLE saveeye_owner;
\ir ../migrations/001_latest_readings.sql
\ir ../migrations/002_edge_provenance.sql
GRANT USAGE ON SCHEMA saveeye TO saveeye_listener;
GRANT SELECT, INSERT, UPDATE ON saveeye.latest_readings TO saveeye_listener;
RESET ROLE;
