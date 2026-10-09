## Devices

Each `[[flow.apps]]` entry lists device constraints in `[[flow.apps.devices]]`: OS and version (`os`), shape (`type`), virtual or physical (`hardware`), or an exact device (`name`). golem turns the constraints into device slots, and the `coverage` strategy decides how many devices run the flow. For each slot it picks a free booted device that matches, boots a shut-down one if no match is booted, and creates one only when nothing matches and `create_if_missing` is set. With no `[[flow.apps.devices]]` at all, the flow runs on whatever is already booted.
