#!/bin/sh
set -eu

mkdir -p /tmp/saveeye-auth
cp /mosquitto/secrets/passwords /tmp/saveeye-auth/passwords
cp /mosquitto/config/acl /tmp/saveeye-auth/acl
chown -R mosquitto:mosquitto /tmp/saveeye-auth
chmod 700 /tmp/saveeye-auth
chmod 600 /tmp/saveeye-auth/passwords /tmp/saveeye-auth/acl
exec mosquitto -c /mosquitto/config/mosquitto.conf
