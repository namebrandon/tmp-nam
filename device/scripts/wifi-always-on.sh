#!/bin/sh
# Factory-test UI is the only Fender screen that enables Wi-Fi.
# Force the radio on so connman auto-joins the last Favorite network
# (AutoConnect=true in /var/lib/connman). Waits for tm-stomp-server first
# (bounded) to preserve the original boot ordering without a systemd
# After= edge (which created an ordering cycle that got this job dropped).
i=0
while [ "$i" -lt 60 ]; do
  if systemctl is-active -q tm-stomp-server.service 2>/dev/null; then
    break
  fi
  i=$((i + 1))
  sleep 1
done
i=0
while [ "$i" -lt 15 ]; do
  dbus-send --system --dest=net.connman --type=method_call \
    /net/connman/technology/wifi \
    net.connman.Technology.SetProperty string:Powered variant:boolean:true \
    >/dev/null 2>&1
  i=$((i + 1))
  sleep 1
done
exit 0
