### `add_media` — Push media to device

Add an image or video file to the device's photo library. golem checks the file's contents: anything other than an image (png, jpeg, gif, webp, heif, bmp, tiff) or a video (mp4, mov) fails with P462. A relative `path` resolves from the directory where you run golem.

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch), or put the file in the device's library before the run.
