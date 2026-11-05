#!/usr/bin/env python3
"""Create local MQTT credentials and listener configuration without secret argv."""

import argparse
import getpass
import os
from pathlib import Path
import re
import subprocess

BROKER_IMAGE = (
    "eclipse-mosquitto:2@sha256:"
    "860aeb5d63ce509508f6dcfb2bc8a1c1dd2bc60555b562b45ce90e52d8dabb0f"
)


def provision(directory, accounts, environment):
    directory = Path(directory).resolve()
    for password in accounts.values():
        if not re.fullmatch(r"[A-Za-z0-9_.~-]{16,}", password):
            raise ValueError("MQTT passwords need 16+ letters, digits or _.~-")
    for values in environment.values():
        for value in values.values():
            if any(c in value for c in "\n\r'"):
                raise ValueError("Percent-encode special characters in configuration values")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise FileExistsError("Secrets already exist; do not overwrite live credentials")
    os.chmod(directory, 0o700)
    for index, (username, password) in enumerate(accounts.items()):
        subprocess.run(
            ["docker", "run", "--rm", "-i", "--user", f"{os.getuid()}:{os.getgid()}",
             "-v", f"{directory}:/secrets", BROKER_IMAGE,
             "mosquitto_passwd", *(["-c"] if index == 0 else []), "/secrets/passwords", username],
            input=f"{password}\n{password}\n", text=True, check=True,
            stdout=subprocess.DEVNULL,
        )
        name = username.removeprefix("saveeye-").removeprefix("sensor-")
        (directory / f"{name}.options").write_text(f"-u {username}\n-P {password}\n")
    for filename, values in environment.items():
        (directory / filename).write_text(
            "".join(f"{key}='{value}'\n" for key, value in values.items())
        )
    for path in directory.iterdir():
        os.chmod(path, 0o600)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("role", choices=("edge", "server"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if args.role == "edge":
        directory = root / "SaveEyeEdgeListenerApp/deploy/secrets"
        device = getpass.getpass("Local saveeye-device password (16+ characters): ")
        listener = getpass.getpass("Local saveeye-listener password (16+ characters): ")
        upstream = getpass.getpass("Server sensor-edge password (same as server): ")
        if not re.fullmatch(r"[A-Za-z0-9_.~-]{16,}", upstream):
            raise ValueError("Upstream password needs 16+ letters, digits or _.~-")
        provision(directory, {"saveeye-device": device, "saveeye-listener": listener}, {
            "edge.env": {
                "SPIN_VARIABLE_MQTT_PASSWORD": listener,
                "SPIN_VARIABLE_UPSTREAM_PASSWORD": upstream,
            },
        })
    else:
        directory = root / "pwrusagesensorbackend/deploy/secrets"
        publisher = getpass.getpass("Server sensor-edge password (share only this with Pi): ")
        backend = getpass.getpass("Server sensor-backend password: ")
        database_url = getpass.getpass("Local PostgreSQL URL (including password and sslmode): ")
        if not database_url.startswith(("postgres://", "postgresql://")):
            raise ValueError("Expected PostgreSQL URL")
        provision(directory, {"sensor-edge": publisher, "sensor-backend": backend}, {
            "backend.env": {
                "SPIN_VARIABLE_MQTT_PASSWORD": backend,
                "SPIN_VARIABLE_DB_URL": database_url,
            },
        })
    print(f"Created protected credentials in {directory}")


if __name__ == "__main__":
    main()
