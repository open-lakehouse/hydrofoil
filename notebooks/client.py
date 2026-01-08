# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "adbc-driver-flightsql>=1.9.0",
#     "deltalake>=1.3.0",
#     "pyarrow",
# ]
# ///

import marimo

__generated_with = "0.18.4"
app = marimo.App()


@app.cell
def _():
    import pyarrow as pa
    import pyarrow.flight

    client = pa.flight.connect("grpc://0.0.0.0:50051")
    return


@app.cell
def _():
    from pprint import pprint

    from adbc_driver_flightsql import DatabaseOptions
    from adbc_driver_flightsql.dbapi import connect

    headers = {"foo": "bar"}

    with connect(
        "grpc://0.0.0.0:50051",
        db_kwargs={
            DatabaseOptions.AUTHORIZATION_HEADER.value: "Bearer <token>",
            DatabaseOptions.TLS_SKIP_VERIFY.value: "true",
            **{
                f"{DatabaseOptions.RPC_CALL_HEADER_PREFIX.value}{k}": v
                for k, v in headers.items()
            },
        },
    ) as conn:
        cursor = conn.cursor()
        cursor.execute("SELECT 1, 2.0, 'Hello, world!'")
        table = cursor.fetch_arrow_table()
        pprint(table)
        cursor.close()
    return


if __name__ == "__main__":
    app.run()
