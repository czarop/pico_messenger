#!/usr/bin/env python3
import struct, base64, ssl, datetime, requests
import paho.mqtt.client as mqtt

BROKER   = "34.148.211.3"     # matches cert CN / IP SAN
PORT     = 8883
CAFILE   = "/etc/mosquitto/certs/ca.crt"
USER     = "czarop"
PASSWORD = "lel22PBP23@01"

# Topics, each with its own meaning — route on topic, never on payload shape.
#   GNSS   : base64-encoded binary LocationPayload (23 bytes)
#   STATUS : plain-text device notices, e.g. "device not moving"
#   WILL   : broker-generated Last Will, published if the device drops without
#            a clean disconnect. NOT the same as a deliberate status notice.
TOPIC_GNSS   = "pico/mqtt/gnss_update"
TOPIC_STATUS = "pico/mqtt/motion_status"
TOPIC_WILL   = "pico/mqtt/status"
TOPIC_DIAG   = "pico/mqtt/diag"

# QoS 1 to match the device's AtLeastOnce publishes.
SUBSCRIPTIONS = [(TOPIC_GNSS, 1), (TOPIC_STATUS, 1), (TOPIC_WILL, 1), (TOPIC_DIAG, 1)]

TG_TOKEN = "8699589319:AAFBpa8sBXzj8dXr5-_mvRwCb258LhMpsNY"
TG_CHAT  = "8633586659"


WIRE_LEN     = 26
GPS_EPOCH    = 315964800      # 1980-01-06 in Unix seconds
LEAP_SECONDS = 18            # UNVERIFIED — see notes

FLAG_MOVING, FLAG_SPEED, FLAG_HEADING = 0b001, 0b010, 0b100
FLAG_CHARGING, FLAG_BATTERY, FLAG_TEMP   = 0b1000, 0b1_0000, 0b10_0000

# HDOP is dimensionless — it describes satellite GEOMETRY only, not measured
# error. To turn it into metres you multiply by the UERE (User Equivalent Range
# Error), the per-satellite ranging error of the receiver/conditions. ~5 m is a
# reasonable nominal for consumer single-band GNSS.
#
#     horizontal error (1-sigma, metres) ~= HDOP x UERE
#
# So the figure reported is an ESTIMATE derived from geometry, not a measurement.
# Real error is often worse near buildings (multipath), which HDOP cannot see.
UERE_M = 5.0


def accuracy_text(hdop):
    """Human-readable horizontal accuracy estimate from HDOP."""
    if hdop <= 0:
        return "accuracy unknown"
    # Standard DOP quality bands.
    if hdop < 2:
        rating = "excellent"
    elif hdop < 5:
        rating = "good"
    elif hdop < 10:
        rating = "moderate"
    elif hdop < 20:
        rating = "fair"
    else:
        rating = "poor"
    return f"\u00b1{hdop * UERE_M:.0f} m ({rating})"


def tg(method, **data):
    try:
        requests.post(f"https://api.telegram.org/bot{TG_TOKEN}/{method}",
                      data=data, timeout=10)
    except Exception as e:
        # A Telegram outage must not kill the bridge; the next message retries.
        print("telegram send failed:", e)


def decode(b64):
    raw = base64.b64decode(b64)
    if len(raw) != WIRE_LEN:
        return None
    (lat, lon, wk, tow, alt, spd, hdg, hdop, vdop,
     soc, temp, fl) = struct.unpack('<iiHIhHHBBBhB', raw)
    unix = GPS_EPOCH + wk * 604800 + tow / 1000.0 - LEAP_SECONDS
    return {
        "lat": lat / 1e6, "lon": lon / 1e6,
        "utc": datetime.datetime.fromtimestamp(unix, datetime.UTC),
        "alt_m": alt,
        "speed_mps": spd / 100 if fl & FLAG_SPEED else None,
        "heading_deg": hdg / 10 if fl & FLAG_HEADING else None,
        "hdop": hdop / 10, "vdop": vdop / 10,
        "moving": bool(fl & FLAG_MOVING),
        # A clear validity bit means "no reading", which is distinct from a zero
        # value -- 0% battery and 0.0 C are both legitimate.
        "battery_pct": soc if fl & FLAG_BATTERY else None,
        "charging": bool(fl & FLAG_CHARGING) if fl & FLAG_BATTERY else None,
        "temp_c": temp / 10 if fl & FLAG_TEMP else None,
    }


def handle_gnss(payload):
    if payload == "hello":          # current firmware test payload
        return
    try:
        d = decode(payload)
    except Exception as e:
        print("decode fail:", e, repr(payload)); return
    if d is None:
        print("non-payload:", repr(payload)); return

    # No timestamp line: Telegram already stamps every message with its receive
    # time, and that is within a second or two of the fix.
    parts = [f"📍 {d['lat']:.6f}, {d['lon']:.6f}",
             f"⛰ {d['alt_m']} m",
             f"🎯 {accuracy_text(d['hdop'])}"]
    if d["speed_mps"]  is not None: parts.append(f"🏃 {d['speed_mps']:.1f} m/s")
    if d["heading_deg"] is not None: parts.append(f"🧭 {d['heading_deg']:.0f}°")
    if d["temp_c"]      is not None: parts.append(f"🌡 {d['temp_c']:.1f}°C")
    if d["battery_pct"] is not None:
        parts.append(f"🔋 {d['battery_pct']}%" + (" ⚡charging" if d["charging"] else ""))
    parts.append("moving" if d["moving"] else "stationary")

    tg("sendLocation", chat_id=TG_CHAT, latitude=d["lat"], longitude=d["lon"])
    tg("sendMessage",  chat_id=TG_CHAT, text="\n".join(parts))
    # Still logged (not sent) so the decoded GPS time can be compared against
    # wall clock — the LEAP_SECONDS value above is unverified.
    print(f"sent: {d['lat']:.6f},{d['lon']:.6f} "
          f"fix_utc={d['utc']:%Y-%m-%d %H:%M:%S} hdop={d['hdop']} vdop={d['vdop']} "
          f"batt={d['battery_pct']} temp={d['temp_c']}")


def handle_status(payload, retained):
    if not payload:
        return
    # Retained messages arrive on every reconnect, so mark them rather than
    # letting an old notice look like a fresh event.
    stamp = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%d %H:%M:%S")
    prefix = "😴" if "not moving" in payload.lower() else "ℹ️"
    age = " (retained)" if retained else ""
    tg("sendMessage", chat_id=TG_CHAT,
       text=f"{prefix} {payload}{age}\n🕒 {stamp} UTC")
    print("status:", payload, "retained" if retained else "")


def handle_will(payload, retained):
    if not payload:
        return
    stamp = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%d %H:%M:%S")
    age = " (retained)" if retained else ""
    # The will fires on an UNCLEAN disconnect — the device dropped without
    # saying goodbye, which during a long sleep may just mean keepalive expiry.
    tg("sendMessage", chat_id=TG_CHAT,
       text=f"⚠️ device link: {payload}{age}\n🕒 {stamp} UTC")
    print("will:", payload, "retained" if retained else "")


def handle_diag(payload, retained):
    # Self-health CSV from the device, published once per successful cycle, e.g.
    #   cyc=42 skip=3 cfail=1 rst=0 psm=2 rsrp=-104
    # cyc = cycles attempted, skip = cycles that published no location (the miss
    # rate is skip/cyc), cfail = MQTT connect failures, rst = modem resets,
    # psm = PSM-entry skips, rsrp = signal (na = no measurable signal).
    # Delivered only on good cycles, but the counters are cumulative, so a rise
    # in skip between two reports means a cycle was missed in between.
    if not payload:
        return
    stamp = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%d %H:%M:%S")
    age = " (retained)" if retained else ""
    # Compact CSV from the device. Legend:
    #   c=cycles s=skipped f=connect-fails r=resets p=psm-skips
    #   d=resends q=signal(RSRP dBm or na). Miss rate = s/c.
    tg("sendMessage", chat_id=TG_CHAT,
       text=f"📊 {payload}{age}\n🕒 {stamp} UTC")
    print("diag:", payload, "retained" if retained else "")


def on_connect(c, u, flags, rc, props=None):
    print("connected rc", rc)
    c.subscribe(SUBSCRIPTIONS)
    print("subscribed:", [t for t, _ in SUBSCRIPTIONS])


def on_message(c, u, msg):
    payload = msg.payload.decode(errors="replace").strip()
    retained = bool(getattr(msg, "retain", False))

    if msg.topic == TOPIC_GNSS:
        handle_gnss(payload)
    elif msg.topic == TOPIC_STATUS:
        handle_status(payload, retained)
    elif msg.topic == TOPIC_WILL:
        handle_will(payload, retained)
    elif msg.topic == TOPIC_DIAG:
        handle_diag(payload, retained)
    else:
        print("unrouted topic:", msg.topic, repr(payload))


cli = mqtt.Client(client_id="telegram-bridge", protocol=mqtt.MQTTv311)
cli.username_pw_set(USER, PASSWORD)
cli.tls_set(CAFILE, tls_version=ssl.PROTOCOL_TLS_CLIENT)
cli.on_connect = on_connect
cli.on_message = on_message
cli.connect(BROKER, PORT, keepalive=60)
cli.loop_forever()