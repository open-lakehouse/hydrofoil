-- Create the databases Unity Catalog, MLflow, and Marquez expect.
-- The default `postgres` database and the POSTGRES_USER role already exist
-- (created by the postgres image entrypoint). This runs once on a fresh volume.
CREATE DATABASE unitycatalog;
CREATE DATABASE mlflow;

-- Marquez ships a baked-in marquez.dev.yml that hardcodes db user/password/name
-- to `marquez` (it only interpolates POSTGRES_HOST/POSTGRES_PORT). So give it a
-- dedicated `marquez` role + database that match those hardcoded credentials.
CREATE ROLE marquez WITH LOGIN PASSWORD 'marquez';
CREATE DATABASE marquez OWNER marquez;
