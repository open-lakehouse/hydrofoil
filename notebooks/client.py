# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "adbc-driver-flightsql==1.7.0",
#     "deltalake==1.1.4",
#     "pyarrow",
# ]
# ///

import marimo

__generated_with = "0.14.17"
app = marimo.App()


@app.cell
def _():
    import pyarrow as pa
    import pyarrow.flight

    client = pa.flight.connect("grpc://0.0.0.0:50051")

    client.handshake()

    return


@app.cell
def _():
    from adbc_driver_flightsql import DatabaseOptions
    from adbc_driver_flightsql.dbapi import connect
    from pprint import pprint

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
        }
    ) as conn:
        info = conn.adbc_get_info()
        pprint(info)
        types = conn.adbc_get_table_types()
        pprint(types)

    return


@app.cell
def _():
    import pyarrow.parquet as pq
    return (pq,)


@app.cell
def _(pq):
    pq.read_table("/Users/robert.pack/code/delta-rs/crates/test/tests/data/table-with-domain-metadata/_delta_log/00000000000000000108.checkpoint.parquet")
    return


if __name__ == "__main__":
    app.run()
