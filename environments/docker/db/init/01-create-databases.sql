-- Create the databases Unity Catalog and MLflow expect.
-- The default `postgres` database and the POSTGRES_USER role already exist
-- (created by the postgres image entrypoint). This runs once on a fresh volume.
CREATE DATABASE unitycatalog;
CREATE DATABASE mlflow;
