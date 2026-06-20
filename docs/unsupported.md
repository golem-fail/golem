# Unsupported

*Cracks in the clay.*

← [Back to README](../README.md)

- **Device orientation control.** Programmatic device rotation is not supported on either platform. iOS 16+ fenced off the cross-process rotation APIs (`UIDevice.setValue("orientation")` is deprecated; the supported `UIWindowScene.requestGeometryUpdate` is in-process only); Android emulator's `user_rotation` setting is overridden by the accelerometer simulator and unreliable. The [`rotate`](actions-reference.md#rotate--rotate-gesture) action exists only as a multi-touch *gesture* (`{ action = "rotate", on = ..., rotation = 90.0 }`) — there is no device-orientation variant.
