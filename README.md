This is a little program for proxying and summarying the information from an [Enphase Envoy](https://enphase.com/installers/communication) solar gateway. It polls the gateway every few minutes and stores the latest value, which it emits on `/metrics.json`.

Configuration:

 - `ENVOY_JWT`: An authentication token, which you can get by hitting `https://enlighten.enphaseenergy.com/entrez-auth-token?serial_num=YOUR_SERIAL_NUMBER` while logged in to the Enlighten app
 - `ENVOY_HOST`: The base URL of the Envoy system; defaults to https://envoy.local

Security note: The envoy uses HTTPS but makes up a totally nonsense certificate (self-signed, expires in the past, no SAN, CN is the serial number). Much MITMing could occur here.
