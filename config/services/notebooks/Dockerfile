FROM ghcr.io/marimo-team/marimo:latest-sql

COPY ./config/services/notebooks/requirements.txt /requirements.txt
RUN uv pip install -r /requirements.txt

WORKDIR /home
