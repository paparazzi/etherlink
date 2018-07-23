#!/bin/bash
# Run on GCS
# local iface has addr of 192.168.69.1, remote iface has 192.168.69.2
# To view/change MTU settings of a device:
#  ip link show | grep tap1 | grep mtu
sudo ip tuntap add dev tap1 mode tap
sudo ip addr add 192.168.69.1/24 broadcast 192.168.69.255 dev tap1
sudo ip link set tap1 up
sudo ip link set tap1 mtu 247
