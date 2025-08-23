This is a little program for proxying and summarying the information from an [Enphase Envoy](https://enphase.com/installers/communication) solar gateway. It polls the gateway every few minutes and stores the latest value, which it emits on `/metrics.json`. It also emits a subset of these metrics in the prometheus format at `/metrics`.

Example output:

```json
{
  "last_update": "2025-08-23T16:05:57Z",
  "battery_soc": 44,
  "pv_mw": 1273179,
  "storage_mw": -714365,
  "grid_mw": 42510,
  "load_mw": 601324,
  "production_mwh_today": 989000,
  "consumption_mwh_today": 4401000,
  "battery_capacity": 15000,
  "num_batteries": 3,
  "history": {
    "pv_mw": {
      "hour": {
        "average": 1308440,
        "count": 6,
        "max": 1332192,
        "min": 1273179
      },
      "day": {
        "average": 1179368,
        "count": 14,
        "max": 1332192,
        "min": 833591
      },
      "week": {
        "average": 1179368,
        "count": 14,
        "max": 1332192,
        "min": 833591
      },
      "last_24h": {
        "2025-08-23T15:00:00Z": 1082564,
        "2025-08-23T16:00:00Z": 1308440
      }
    },
    "grid_mw": {
      "hour": {
        "average": 25841,
        "count": 6,
        "max": 42510,
        "min": 9492
      },
      "day": {
        "average": 31819,
        "count": 14,
        "max": 147346,
        "min": -8392
      },
      "week": {
        "average": 31819,
        "count": 14,
        "max": 147346,
        "min": -8392
      },
      "last_24h": {
        "2025-08-23T15:00:00Z": 36302,
        "2025-08-23T16:00:00Z": 25841
      }
    },
    "load_mw": {
      "hour": {
        "average": 769313,
        "count": 6,
        "max": 828553,
        "min": 601324
      },
      "day": {
        "average": 695950,
        "count": 14,
        "max": 837114,
        "min": 567766
      },
      "week": {
        "average": 695950,
        "count": 14,
        "max": 837114,
        "min": 567766
      },
      "last_24h": {
        "2025-08-23T15:00:00Z": 640928,
        "2025-08-23T16:00:00Z": 769313
      }
    },
    "storage_mw": {
      "hour": {
        "average": -564968,
        "count": 6,
        "max": -501776,
        "min": -714365
      },
      "day": {
        "average": -515237,
        "count": 14,
        "max": -259865,
        "min": -714365
      },
      "week": {
        "average": -515237,
        "count": 14,
        "max": -259865,
        "min": -714365
      },
      "last_24h": {
        "2025-08-23T15:00:00Z": -477939,
        "2025-08-23T16:00:00Z": -564968
      }
    }
  }
}
```

Configuration:

 - `ENVOY_JWT`: An authentication token, which you can get by hitting `https://enlighten.enphaseenergy.com/entrez-auth-token?serial_num=YOUR_SERIAL_NUMBER` while logged in to the Enlighten app
 - `ENVOY_HOST`: The base URL of the Envoy system; defaults to https://envoy.local

Security note: The envoy uses HTTPS but makes up a totally nonsense certificate (self-signed, expires in the past, no SAN, CN is the serial number). Much MITMing could occur here. You should run this on the same LAN as your Envoy gateway.
