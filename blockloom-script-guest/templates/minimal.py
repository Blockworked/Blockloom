"""Minimal Python guest: the same contract as minimal.rs and minimal.c.
`start` says hello, `tick` reads its x and steps +1 along it, `on_event`
answers. Built against the frozen world with componentize-py:

    pip install componentize-py
    componentize-py -d wit -w script componentize minimal -o minimal.wasm

then put minimal.wasm under assets/scripts and add it as the actor's script.
"""
from wit_world.imports import acts, sensors

# abi.rs verb numbers (blockloom-core/src/script/abi.rs).
READ_POSITION = 1
ACT_CHANGE_POSITION = 3
ACT_SAY = 10


class Entry:
    def start(self):
        acts.act(ACT_SAY, "hello from wasm", "", "", [])

    def tick(self, dt):
        sensors.read(READ_POSITION, "", "", 0.0)
        acts.act(ACT_CHANGE_POSITION, "", "", "", [0.0, 1.0])

    def frame(self, dt):
        pass

    def ui(self, dt):
        pass

    def stop(self):
        pass

    def destroy(self):
        pass

    def on_event(self, kind, n0, n1, n2, n3):
        acts.act(ACT_SAY, "event heard", "", "", [])
