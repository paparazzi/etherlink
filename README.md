# etherlink
Ethernet over Pprzlink

Provides transparent connection between a GCS linux computer and a Mission/Copilot computer (also a linux computer).
Etherlink provides a tap interface on each side, passing network traffic as `PAYLOAD_COMMAND` message to the autopilot.

## Howto Autopilot
1. make sure you have latest `master` version of paparazzi
2. add `copilot` module with enabled PAYLOAD_COMMAND forwarding:
```
<module name="copilot">
  <define name="FORWARD_PAYLOAD" value="TRUE"/>
</module>
```
3. you need a working serial connection to your [Mission computer/Copilot](https://wiki.paparazziuav.org/wiki/Mission_computer)
4. Clean, Build & Upload, make sure you have telemetry from the Autopilot

## Howto GCS
1. `cargo build`
2. run `./init_gcs.sh` - that will create a tap interface `tap1` with IP addr `192.168.69.1`
3. run a regular pprz session with `link` etc.
4. `cargo run` - will start etherlink

# Howto Copilot
1. `cargo build` (has to be done while online)
2. run `./init_copilot.sh` - that will create a tap interface `tap1` with IP addr `192.168.69.2`
3. run `link` program
4. `cargo run` to start etherlink

Now you can treat the `tap1` interface as a regular interface, most likely you will want to send UDP/TCP data (which requires
an application on the other side to listen at a particular port, otherwise an ICMP message "Destination unreachable" will 
be emitted). But for start, you can just ping your Copilot:-)
