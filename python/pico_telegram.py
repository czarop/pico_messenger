#!/usr/bin/env python3
import struct, base64, ssl, datetime, requests
import paho.mqtt.client as mqtt

BROKER   = "34.148.211.3"     # matches cert CN / IP SAN
PORT     = 8883
CAFILE   = "/etc/mosquitto/certs/ca.crt"
USER     = "czarop"
PASSWORD = "lel22PBP23@01"
TOPIC    = "pico/mqtt/gnss_update"

TG_TOKEN = "8699589319:AAFBpa8sBXzj8dXr5-_mvRwCb258LhMpsNY"
TG_CHAT  = "8633586659"

WIRE_LEN     = 23
GPS_EPOCH    = 315964800      # 1980-01-06 in Unix seconds
LEAP_SECONDS = 18            # UNVERIFIED — see notes

FLAG_MOVING, FLAG_SPEED, FLAG_HEADING = 0b001, 0b010, 0b100

def tg(method, **data):
    requests.post(f"https://api.telegram.org/bot{TG_TOKEN}/{method}", data=data, timeout=10)

def decode(b64):
    raw = base64.b64decode(b64)
    if len(raw) != WIRE_LEN:
        return None
    (lat, lon, wk, tow, alt, spd, hdg, hdop, vdop, fl) = struct.unpack('<iiHIhHHBBB', raw)
    unix = GPS_EPOCH + wk * 604800 + tow / 1000.0 - LEAP_SECONDS
    return {
        "lat": lat / 1e6, "lon": lon / 1e6,
        "utc": datetime.datetime.fromtimestamp(unix, datetime.UTC),
        "alt_m": alt,
        "speed_mps": spd / 100 if fl & FLAG_SPEED else None,
        "heading_deg": hdg / 10 if fl & FLAG_HEADING else None,
        "hdop": hdop / 10, "vdop": vdop / 10,
        "moving": bool(fl & FLAG_MOVING),
    }

def on_connect(c, u, flags, rc, props=None):
    print("connected rc", rc); c.subscribe(TOPIC)

def on_message(c, u, msg):
    payload = msg.payload.decode(errors="replace").strip()
    if payload == "hello":          # current firmware test payload
        return
    try:
        d = decode(payload)
    except Exception as e:
        print("decode fail:", e, repr(payload)); return
    if d is None:
        print("non-payload:", repr(payload)); return

    parts = [f"📍 {d['lat']:.6f}, {d['lon']:.6f}",
             f"🕒 {d['utc']:%Y-%m-%d %H:%M:%S} UTC",
             f"⛰ {d['alt_m']} m   HDOP {d['hdop']}"]
    if d["speed_mps"]  is not None: parts.append(f"🏃 {d['speed_mps']:.1f} m/s")
    if d["heading_deg"] is not None: parts.append(f"🧭 {d['heading_deg']:.0f}°")
    parts.append("moving" if d["moving"] else "stationary")

    tg("sendLocation", chat_id=TG_CHAT, latitude=d["lat"], longitude=d["lon"])
    tg("sendMessage",  chat_id=TG_CHAT, text="\n".join(parts))
    print("sent:", d["lat"], d["lon"])

cli = mqtt.Client(client_id="telegram-bridge", protocol=mqtt.MQTTv311)
cli.username_pw_set(USER, PASSWORD)
cli.tls_set(CAFILE, tls_version=ssl.PROTOCOL_TLS_CLIENT)
cli.on_connect = on_connect
cli.on_message = on_message
cli.connect(BROKER, PORT, keepalive=60)
cli.loop_forever()