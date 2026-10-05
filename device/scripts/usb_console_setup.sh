#!/bin/sh
# usb_console_setup.sh — idempotently add a CDC-ACM function to the TMP's
# ARM USB gadget (configfs gadget0) and produce /dev/ttyGS0.
#
# Installed at /usr/local/bin/usb_console_setup.sh; run by usb-console.service
# at boot. The stock systemd gadget units create gadget0 with hid.hid0 and
# midi.usb0. The SD-root patch stages acm.usb0 from the MIDI setup script before
# the stock central bind, so the host sees all three functions at power-on.
#
# CRITICAL: never unbind/rebind the UDC here. Doing so while a shell is running
# tears down ttyGS0 (u_serial deregisters on unbind). Since the service restarts
# on shell exit, an unconditional rebind would create a death loop. This helper
# only supplies a missing bind if the stock central binder has not run yet.
#
# Safe to re-run: all steps are existence/state-checked.

GADGET=/sys/kernel/config/usb_gadget/gadget0
UDC_DEV=e6590000.usb
FUNC=acm.usb0

# wait for the stock setup service to create the gadget
i=0
while [ ! -d "$GADGET/functions" ]; do
  i=$((i+1))
  [ $i -ge 60 ] && exit 1
  sleep 1
done

modprobe usb_f_acm 2>/dev/null       # ok if already loaded/builtin

if [ ! -e "$GADGET/functions/$FUNC" ]; then
  mkdir -p "$GADGET/functions/$FUNC"
fi

if [ ! -L "$GADGET/configs/config.1/$FUNC" ]; then
  ln -s "$GADGET/functions/$FUNC" "$GADGET/configs/config.1/$FUNC"
fi

current=$(cat "$GADGET/UDC" 2>/dev/null)
if [ -z "$current" ]; then
  echo "$UDC_DEV" > "$GADGET/UDC"
fi

# the tty appears once the function is allocated
i=0
while [ ! -e /dev/ttyGS0 ]; do
  i=$((i+1))
  [ $i -ge 15 ] && exit 1
  sleep 1
done
exit 0
