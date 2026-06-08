package fail.golem.companion;

import android.app.UiAutomation;
import android.graphics.Bitmap;
import android.os.ParcelFileDescriptor;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

public class CompanionServer {

    private static final int DEFAULT_PORT = 8223;
    /** Inactivity timeout — server exits after this duration with no requests. */
    private static final long INACTIVITY_TIMEOUT_MS = 5 * 60 * 60 * 1000L; // 5 hours
    /** Per-call deadline on UiAutomation operations. Normal returns are
     *  100-300ms; if the internal IPC wedges the call CAN hang forever.
     *  3s separates "slow tail under load" from "permanently stuck". */
    private static final long UI_CALL_TIMEOUT_MS = 3_000L;
    /** Self-suicide threshold. Persistent null returns over a sustained
     *  window indicate the UiAutomation handle is wedged (singleton can
     *  survive but lose its accessibility connection). Exiting triggers
     *  instrumentation auto-restart — the host's recovery sees the
     *  companion-unresponsive signal and re-registers with a fresh
     *  handle. Conservative thresholds — false-positive suicide costs
     *  ~10s reboot vs forever-wedged. */
    private static final int STALENESS_NULL_THRESHOLD = 20;
    private static final long STALENESS_WINDOW_MS = 60_000L;

    /** Single-thread executor for UiAutomation calls with timeout.
     *  Daemon so it doesn't keep the JVM alive at shutdown. */
    private static final ExecutorService UI_EXECUTOR =
        Executors.newSingleThreadExecutor(r -> {
            Thread t = new Thread(r, "golem-ui-bounded");
            t.setDaemon(true);
            return t;
        });
    private static final AtomicInteger CONSECUTIVE_NULLS = new AtomicInteger(0);
    private static volatile long LAST_UI_SUCCESS_MS = System.currentTimeMillis();

    private final UiAutomation uiAutomation;
    private final int port;
    /** ADB serial passed from the host (e.g. "emulator-5554"). */
    private final String deviceSerial;
    private volatile long lastRequestTime = System.currentTimeMillis();

    public CompanionServer(UiAutomation uiAutomation) {
        this(uiAutomation, DEFAULT_PORT, null);
    }

    public CompanionServer(UiAutomation uiAutomation, int port, String deviceSerial) {
        this.uiAutomation = uiAutomation;
        this.port = port;
        this.deviceSerial = deviceSerial != null ? deviceSerial : "unknown";
    }

    /** Optional callback to re-register and get a new port when bind fails. */
    interface PortAllocator {
        int allocatePort() throws Exception;
    }

    private PortAllocator portAllocator;

    public void setPortAllocator(PortAllocator allocator) {
        this.portAllocator = allocator;
    }

    public void start() throws IOException {
        startInactivityWatchdog();
        ServerSocket serverSocket = tryBind(port);
        while (true) {
            Socket client = serverSocket.accept();
            new Thread(() -> {
                try {
                    lastRequestTime = System.currentTimeMillis();
                    handleClient(client);
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    try { client.close(); } catch (IOException ignored) {}
                }
            }).start();
        }
    }

    private void startInactivityWatchdog() {
        Thread watchdog = new Thread(() -> {
            while (true) {
                try {
                    Thread.sleep(60_000); // check every minute
                } catch (InterruptedException e) {
                    return;
                }
                long idle = System.currentTimeMillis() - lastRequestTime;
                if (idle >= INACTIVITY_TIMEOUT_MS) {
                    android.util.Log.i("GolemCompanion",
                        "Shutting down after " + (idle / 3600000) + " hours of inactivity");
                    System.exit(0);
                }
            }
        });
        watchdog.setDaemon(true);
        watchdog.start();
    }

    private void handleClient(Socket client) throws Exception {
        BufferedReader reader = new BufferedReader(
                new InputStreamReader(client.getInputStream(), StandardCharsets.UTF_8));
        String requestLine = reader.readLine();
        if (requestLine == null) return;

        String[] parts = requestLine.split(" ");
        if (parts.length < 2) return;

        String method = parts[0];
        String fullPath = parts[1];
        String path = fullPath.split("\\?")[0];
        Map<String, String> queryParams = parseQueryParams(fullPath);

        // Read headers
        Map<String, String> headers = new HashMap<>();
        String headerLine;
        while ((headerLine = reader.readLine()) != null && !headerLine.isEmpty()) {
            int colon = headerLine.indexOf(':');
            if (colon > 0) {
                headers.put(headerLine.substring(0, colon).trim().toLowerCase(),
                        headerLine.substring(colon + 1).trim());
            }
        }

        // Read body if present
        String body = "";
        if (headers.containsKey("content-length")) {
            int len = Integer.parseInt(headers.get("content-length"));
            char[] buf = new char[len];
            int read = 0;
            while (read < len) {
                int n = reader.read(buf, read, len - read);
                if (n == -1) break;
                read += n;
            }
            body = new String(buf, 0, read);
        }

        OutputStream out = client.getOutputStream();

        try {
            switch (path) {
                case "/health":
                    sendJson(out, 200, new JSONObject()
                        .put("status", "ok")
                        .put("platform", "android")
                        .put("version", "0.6.26")
                        .put("device_name", android.os.Build.MODEL)
                        .put("device_model", android.os.Build.DEVICE)
                        .put("os_version", String.valueOf(android.os.Build.VERSION.SDK_INT))
                        .put("device_id", deviceSerial));
                    break;
                case "/hierarchy":
                    handleHierarchy(out);
                    break;
                case "/tap":
                    handleTap(out, body);
                    break;
                case "/longpress":
                    handleLongPress(out, body);
                    break;
                case "/type":
                    handleType(out, body);
                    break;
                case "/backspace":
                    handleBackspace(out, body);
                    break;
                case "/swipe":
                    handleSwipe(out, body);
                    break;
                case "/pinch":
                    handlePinch(out, body);
                    break;
                case "/gesture":
                    handleGesture(out, body);
                    break;
                case "/screenshot":
                    handleScreenshot(out);
                    break;
                case "/hide-keyboard":
                    handleHideKeyboard(out);
                    break;
                case "/launch":
                    handleLaunch(out, body);
                    break;
                case "/stop":
                    handleStop(out, body);
                    break;
                case "/perf":
                    handlePerf(out, queryParams);
                    break;
                default:
                    sendJson(out, 404, new JSONObject().put("error", "not found"));
                    break;
            }
        } catch (Exception e) {
            sendJson(out, 500, new JSONObject().put("error", e.getMessage()));
        }
    }

    /** Call a UiAutomation operation with a hard per-call deadline and
     *  staleness tracking. Returns null on timeout or null-result.
     *  Persistent nulls trigger {@link System#exit} to force
     *  instrumentation restart — the host re-registers with a fresh
     *  UiAutomation handle. */
    private <T> T callBounded(Callable<T> op, String label) {
        Future<T> f = UI_EXECUTOR.submit(op);
        T result = null;
        try {
            result = f.get(UI_CALL_TIMEOUT_MS, TimeUnit.MILLISECONDS);
        } catch (Exception e) {
            // timeout, interrupted, or execution error — treat as null;
            // cancel so the worker isn't stuck on the wedged call.
            f.cancel(true);
        }
        if (result != null) {
            CONSECUTIVE_NULLS.set(0);
            LAST_UI_SUCCESS_MS = System.currentTimeMillis();
            return result;
        }
        int nulls = CONSECUTIVE_NULLS.incrementAndGet();
        long sinceSuccessMs = System.currentTimeMillis() - LAST_UI_SUCCESS_MS;
        if (nulls >= STALENESS_NULL_THRESHOLD && sinceSuccessMs > STALENESS_WINDOW_MS) {
            System.err.println(
                "[companion] UiAutomation wedged (" + nulls + " consecutive nulls on " +
                label + ", " + (sinceSuccessMs / 1000) + "s since last success) — exiting");
            System.exit(0);
        }
        return null;
    }

    private void handleHierarchy(OutputStream out) throws Exception {
        // Same transient-null behaviour as `takeScreenshot()`: an
        // overlay, animation, or accessibility-connection blip can
        // leave UiAutomation with no active window briefly. Retry
        // before failing — assert_visible callers poll the endpoint
        // for visibility anyway, but a hard 500 here can mask
        // genuinely-visible elements when the companion's snapshot
        // lags the device by a frame or two.
        AccessibilityNodeInfo root = null;
        int rootAttempts = 0;
        while (rootAttempts < 3) {
            root = callBounded(uiAutomation::getRootInActiveWindow, "getRootInActiveWindow");
            if (root != null) break;
            rootAttempts++;
            try { Thread.sleep(100); } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
        }
        if (root == null) {
            sendJson(out, 500, new JSONObject()
                .put("error", "no active window")
                .put("attempts", rootAttempts));
            return;
        }
        try {
            // Parse dumpsys window for keyboard, safe area, cutouts, and corners.
            int keyboardHeight = 0;
            int safeAreaTop = 0;
            int safeAreaBottom = 0;
            int safeAreaLeft = 0;
            int safeAreaRight = 0;
            JSONArray cutouts = new JSONArray();
            JSONArray roundedCorners = new JSONArray();
            try {
                String windowDump = executeShellAndRead("dumpsys window");
                for (String line : windowDump.split("\n")) {
                    // Keyboard: type=ime frame=[0,TOP][W,BOT] visible=true
                    if (keyboardHeight == 0 && line.contains("type=ime")
                            && line.contains("visible=true") && line.contains("frame=[")) {
                        int frameIdx = line.indexOf("frame=[");
                        String frameStr = line.substring(frameIdx + 7);
                        String[] parts = frameStr.split("[\\[\\],]+");
                        if (parts.length >= 4) {
                            int top = Integer.parseInt(parts[1].trim());
                            int bottom = Integer.parseInt(parts[3].trim());
                            if (bottom > top && top > 0) {
                                keyboardHeight = bottom - top;
                            }
                        }
                    }
                    // Status bar: InsetsSource type=statusBars frame=[0,0][W,H]
                    if (safeAreaTop == 0 && line.contains("type=statusBars") && line.contains("frame=[")) {
                        int[] frame = parseInsetsFrame(line);
                        if (frame != null && frame[3] > 0) {
                            safeAreaTop = frame[3]; // bottom of status bar frame
                        }
                    }
                    // Navigation bar: InsetsSource type=navigationBars frame=[0,TOP][W,BOT] visible=true
                    if (safeAreaBottom == 0 && line.contains("type=navigationBars")
                            && line.contains("visible=true") && line.contains("frame=[")) {
                        int[] frame = parseInsetsFrame(line);
                        if (frame != null && frame[3] > frame[1]) {
                            safeAreaBottom = frame[3] - frame[1]; // height of nav bar
                        }
                    }
                    // System gesture zones (back-from-edge, swipe-from-top, etc).
                    // Reported per-side via sideHint=LEFT/RIGHT. Width of the
                    // inset = how far the gesture band extends from that edge —
                    // touches in this band may be interpreted as system gestures
                    // rather than app input. Auto-scroll swipes need to avoid
                    // them, especially the LEFT band where back-from-edge would
                    // intercept a horizontal swipe.
                    if (safeAreaLeft == 0 && line.contains("type=systemGestures")
                            && line.contains("sideHint=LEFT") && line.contains("frame=[")) {
                        int[] frame = parseInsetsFrame(line);
                        if (frame != null && frame[2] > frame[0]) {
                            safeAreaLeft = frame[2] - frame[0];
                        }
                    }
                    if (safeAreaRight == 0 && line.contains("type=systemGestures")
                            && line.contains("sideHint=RIGHT") && line.contains("frame=[")) {
                        int[] frame = parseInsetsFrame(line);
                        if (frame != null && frame[2] > frame[0]) {
                            safeAreaRight = frame[2] - frame[0];
                        }
                    }
                    // Cutouts: mDisplayCutout=DisplayCutout{...boundingRect={Bounds=[Rect(...), ...]}}
                    if (line.contains("mDisplayCutout=") && line.contains("boundingRect=")) {
                        cutouts = parseCutoutBounds(line);
                    }
                    // Corners: mRoundedCorners=RoundedCorners{[RoundedCorner{...}, ...]}
                    if (line.contains("mRoundedCorners=RoundedCorners")) {
                        roundedCorners = parseRoundedCorners(line);
                    }
                }
            } catch (Exception ignored) {
                // Display info detection is best-effort
            }
            JSONObject tree = buildNodeJson(root);
            JSONObject response = new JSONObject();
            response.put("tree", tree);
            response.put("keyboard_height", keyboardHeight);
            response.put("safe_area_top", safeAreaTop);
            response.put("safe_area_bottom", safeAreaBottom);
            response.put("safe_area_left", safeAreaLeft);
            response.put("safe_area_right", safeAreaRight);
            response.put("cutouts", cutouts);
            response.put("rounded_corners", roundedCorners);
            sendJson(out, 200, response);
        } finally {
            root.recycle();
        }
    }

    private void handleTap(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int x = req.getInt("x");
        int y = req.getInt("y");
        executeShell("input tap " + x + " " + y);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleLongPress(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int x = req.getInt("x");
        int y = req.getInt("y");
        long duration = req.optLong("duration_ms", 1500);
        executeShell("input swipe " + x + " " + y + " " + x + " " + y + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleSwipe(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int fromX = req.getInt("from_x");
        int fromY = req.getInt("from_y");
        int toX = req.getInt("to_x");
        int toY = req.getInt("to_y");
        long duration = req.optLong("duration_ms", 300);
        executeShell("input swipe " + fromX + " " + fromY + " " + toX + " " + toY + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    /**
     * Pinch gesture at coordinates.
     * Request: { "x": N, "y": N, "scale": 2.0, "velocity": 5.0 }
     */
    private void handlePinch(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int cx = req.getInt("x");
        int cy = req.getInt("y");
        double scale = req.getDouble("scale");
        double velocity = req.optDouble("velocity", 5.0);
        long durationMs = Math.max(100, (long) (Math.abs(scale - 1.0) / velocity * 1000));

        // Fingers start close together and spread apart for zoom-in (scale > 1),
        // or start apart and come together for zoom-out (scale < 1).
        int startDist = 50; // 50px from center
        int endDist = (int) (startDist * scale);

        int[][] allX = { {cx, cx}, {cx, cx} };
        int[][] allY = { {cy - startDist, cy - endDist}, {cy + startDist, cy + endDist} };
        long[] durations = { durationMs, durationMs };

        injectMultiTouchGesture(allX, allY, durations);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    /**
     * Execute a multi-touch gesture by injecting MotionEvents via shell input commands.
     * Request: { "fingers": [{ "points": [[x,y], ...], "duration_ms": N }, ...] }
     *
     * For single-finger gestures: uses "input swipe" through intermediate points.
     * For multi-finger gestures: uses "input motionevent" to inject raw touch events.
     */
    private void handleGesture(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        JSONArray fingers = req.getJSONArray("fingers");

        if (fingers.length() == 0) {
            sendJson(out, 400, new JSONObject().put("error", "need at least one finger"));
            return;
        }

        // Parse all finger paths
        int[][] allX = new int[fingers.length()][];
        int[][] allY = new int[fingers.length()][];
        long[] durations = new long[fingers.length()];

        for (int f = 0; f < fingers.length(); f++) {
            JSONObject finger = fingers.getJSONObject(f);
            JSONArray points = finger.getJSONArray("points");
            durations[f] = finger.optLong("duration_ms", 300);

            if (points.length() < 2) {
                sendJson(out, 400, new JSONObject().put("error", "each finger needs at least 2 points"));
                return;
            }

            allX[f] = new int[points.length()];
            allY[f] = new int[points.length()];
            for (int i = 0; i < points.length(); i++) {
                JSONArray pt = points.getJSONArray(i);
                allX[f][i] = pt.getInt(0);
                allY[f][i] = pt.getInt(1);
            }
        }

        // Chained `input swipe` lifts/downs between segments, breaking
        // the continuous path that gesture-tracking widgets (drawing,
        // L-swipe grids, drag handles) require. Always inject raw
        // MotionEvents so DOWN → multi-MOVE → UP stays a single
        // gesture for any finger count.
        injectMultiTouchGesture(allX, allY, durations);

        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    /**
     * Inject a multi-finger gesture using Instrumentation.sendPointerSync().
     * Creates MotionEvents with multiple pointer IDs for simultaneous touches.
     */
    private void injectMultiTouchGesture(int[][] allX, int[][] allY, long[] durations) throws Exception {
        int fingerCount = allX.length;
        // Use the max number of points across all fingers for interpolation steps
        int maxPoints = 0;
        for (int[] xs : allX) maxPoints = Math.max(maxPoints, xs.length);
        long maxDuration = 0;
        for (long d : durations) maxDuration = Math.max(maxDuration, d);

        int steps = Math.max(maxPoints, 20); // at least 20 steps for smooth gesture
        long stepDelay = maxDuration / steps;

        // Build PointerProperties and PointerCoords for each finger
        android.view.MotionEvent.PointerProperties[] props =
            new android.view.MotionEvent.PointerProperties[fingerCount];
        for (int f = 0; f < fingerCount; f++) {
            props[f] = new android.view.MotionEvent.PointerProperties();
            props[f].id = f;
            props[f].toolType = android.view.MotionEvent.TOOL_TYPE_FINGER;
        }

        long downTime = android.os.SystemClock.uptimeMillis();

        // ACTION_DOWN for first finger
        android.view.MotionEvent.PointerCoords[] coords =
            new android.view.MotionEvent.PointerCoords[fingerCount];
        for (int f = 0; f < fingerCount; f++) {
            coords[f] = new android.view.MotionEvent.PointerCoords();
            coords[f].x = allX[f][0];
            coords[f].y = allY[f][0];
            coords[f].pressure = 1.0f;
            coords[f].size = 1.0f;
        }

        // First finger down
        android.view.MotionEvent downEvent = android.view.MotionEvent.obtain(
            downTime, downTime, android.view.MotionEvent.ACTION_DOWN,
            1, new android.view.MotionEvent.PointerProperties[]{props[0]},
            new android.view.MotionEvent.PointerCoords[]{coords[0]},
            0, 0, 1.0f, 1.0f, 0, 0, android.view.InputDevice.SOURCE_TOUCHSCREEN, 0);
        uiAutomation.injectInputEvent(downEvent, true);
        downEvent.recycle();

        // Additional fingers down (ACTION_POINTER_DOWN)
        for (int f = 1; f < fingerCount; f++) {
            android.view.MotionEvent.PointerProperties[] activeProps =
                new android.view.MotionEvent.PointerProperties[f + 1];
            android.view.MotionEvent.PointerCoords[] activeCoords =
                new android.view.MotionEvent.PointerCoords[f + 1];
            System.arraycopy(props, 0, activeProps, 0, f + 1);
            System.arraycopy(coords, 0, activeCoords, 0, f + 1);

            int action = android.view.MotionEvent.ACTION_POINTER_DOWN |
                (f << android.view.MotionEvent.ACTION_POINTER_INDEX_SHIFT);
            long eventTime = android.os.SystemClock.uptimeMillis();
            android.view.MotionEvent ptrDown = android.view.MotionEvent.obtain(
                downTime, eventTime, action, f + 1, activeProps, activeCoords,
                0, 0, 1.0f, 1.0f, 0, 0, android.view.InputDevice.SOURCE_TOUCHSCREEN, 0);
            uiAutomation.injectInputEvent(ptrDown, true);
            ptrDown.recycle();
        }

        // Move all fingers through interpolated positions
        for (int s = 1; s <= steps; s++) {
            float t = (float) s / steps;
            for (int f = 0; f < fingerCount; f++) {
                // Interpolate position along this finger's path
                float pathPos = t * (allX[f].length - 1);
                int segIdx = Math.min((int) pathPos, allX[f].length - 2);
                float segT = pathPos - segIdx;
                coords[f].x = allX[f][segIdx] + (allX[f][segIdx + 1] - allX[f][segIdx]) * segT;
                coords[f].y = allY[f][segIdx] + (allY[f][segIdx + 1] - allY[f][segIdx]) * segT;
            }

            long eventTime = android.os.SystemClock.uptimeMillis();
            android.view.MotionEvent moveEvent = android.view.MotionEvent.obtain(
                downTime, eventTime, android.view.MotionEvent.ACTION_MOVE,
                fingerCount, props, coords,
                0, 0, 1.0f, 1.0f, 0, 0, android.view.InputDevice.SOURCE_TOUCHSCREEN, 0);
            uiAutomation.injectInputEvent(moveEvent, true);
            moveEvent.recycle();

            Thread.sleep(stepDelay);
        }

        // Lift fingers in reverse order
        for (int f = fingerCount - 1; f >= 1; f--) {
            android.view.MotionEvent.PointerProperties[] activeProps =
                new android.view.MotionEvent.PointerProperties[f + 1];
            android.view.MotionEvent.PointerCoords[] activeCoords =
                new android.view.MotionEvent.PointerCoords[f + 1];
            System.arraycopy(props, 0, activeProps, 0, f + 1);
            System.arraycopy(coords, 0, activeCoords, 0, f + 1);

            int action = android.view.MotionEvent.ACTION_POINTER_UP |
                (f << android.view.MotionEvent.ACTION_POINTER_INDEX_SHIFT);
            long eventTime = android.os.SystemClock.uptimeMillis();
            android.view.MotionEvent ptrUp = android.view.MotionEvent.obtain(
                downTime, eventTime, action, f + 1, activeProps, activeCoords,
                0, 0, 1.0f, 1.0f, 0, 0, android.view.InputDevice.SOURCE_TOUCHSCREEN, 0);
            uiAutomation.injectInputEvent(ptrUp, true);
            ptrUp.recycle();
        }

        // Last finger up
        long eventTime = android.os.SystemClock.uptimeMillis();
        android.view.MotionEvent upEvent = android.view.MotionEvent.obtain(
            downTime, eventTime, android.view.MotionEvent.ACTION_UP,
            1, new android.view.MotionEvent.PointerProperties[]{props[0]},
            new android.view.MotionEvent.PointerCoords[]{coords[0]},
            0, 0, 1.0f, 1.0f, 0, 0, android.view.InputDevice.SOURCE_TOUCHSCREEN, 0);
        uiAutomation.injectInputEvent(upEvent, true);
        upEvent.recycle();
    }

    private void handleType(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String text = req.getString("text");
        // Split on newlines — type each line separately with Enter between them.
        // Android's `input text` doesn't support newline characters.
        String[] lines = text.split("\n", -1);
        for (int i = 0; i < lines.length; i++) {
            if (!lines[i].isEmpty()) {
                executeShell("input text " + escapeForInputText(lines[i]));
            }
            if (i < lines.length - 1) {
                executeShell("input keyevent KEYCODE_ENTER");
            }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleBackspace(OutputStream out, String body) throws Exception {
        JSONObject req = body.isEmpty() ? new JSONObject() : new JSONObject(body);
        int count = req.optInt("count", 1);
        for (int i = 0; i < count; i++) {
            executeShell("input keyevent DEL");
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleScreenshot(OutputStream out) throws Exception {
        // UiAutomation.takeScreenshot() can return null transiently
        // when the WindowManager snapshot pipeline is busy (a system
        // overlay flashed up, ongoing animation, accessibility
        // connection blip). Sweep runs surfaced this regularly. Retry
        // a few times with short sleeps before declaring failure —
        // happy-path callers add ~0ms; the bad path costs ~200ms
        // before reporting 500.
        Bitmap bitmap = null;
        int attempts = 0;
        while (attempts < 3) {
            bitmap = callBounded(uiAutomation::takeScreenshot, "takeScreenshot");
            if (bitmap != null) break;
            attempts++;
            try { Thread.sleep(100); } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
        }
        if (bitmap == null) {
            sendJson(out, 500, new JSONObject()
                .put("error", "screenshot failed")
                .put("attempts", attempts));
            return;
        }
        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, baos);
        bitmap.recycle();
        byte[] png = baos.toByteArray();

        String header = "HTTP/1.1 200 OK\r\n"
                + "Content-Type: image/png\r\n"
                + "Content-Length: " + png.length + "\r\n"
                + "Connection: close\r\n\r\n";
        out.write(header.getBytes(StandardCharsets.UTF_8));
        out.write(png);
        out.flush();
    }

    private void handleHideKeyboard(OutputStream out) throws Exception {
        // KEYCODE_BACK is the universal "dismiss IME" gesture on Android,
        // but if the IME is already hidden the same key event propagates
        // to the activity and finishes it — exiting the app under test.
        // Probe `dumpsys input_method` for `mInputShown=true` and only
        // press BACK when there's actually a keyboard to dismiss.
        String dump = executeShellAndRead("dumpsys input_method");
        if (dump != null && dump.contains("mInputShown=true")) {
            executeShell("input keyevent KEYCODE_BACK");
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private JSONObject buildNodeJson(AccessibilityNodeInfo node) throws Exception {
        JSONObject json = new JSONObject();
        CharSequence cls = node.getClassName();
        json.put("class", cls != null ? cls.toString() : "");
        CharSequence text = node.getText();
        json.put("text", text != null ? text.toString() : "");
        CharSequence desc = node.getContentDescription();
        json.put("contentDescription", desc != null ? desc.toString() : "");
        json.put("clickable", node.isClickable());
        json.put("enabled", node.isEnabled());
        json.put("focused", node.isFocused());
        json.put("scrollable", node.isScrollable());
        json.put("selected", node.isSelected());
        json.put("checked", node.isChecked());

        android.graphics.Rect bounds = new android.graphics.Rect();
        node.getBoundsInScreen(bounds);
        JSONObject boundsJson = new JSONObject();
        boundsJson.put("left", bounds.left);
        boundsJson.put("top", bounds.top);
        boundsJson.put("right", bounds.right);
        boundsJson.put("bottom", bounds.bottom);
        json.put("bounds", boundsJson);

        // Compute visible bounds by intersecting with all ancestor bounds
        android.graphics.Rect visibleBounds = getVisibleBounds(node);
        JSONObject visibleBoundsJson = new JSONObject();
        visibleBoundsJson.put("left", visibleBounds.left);
        visibleBoundsJson.put("top", visibleBounds.top);
        visibleBoundsJson.put("right", visibleBounds.right);
        visibleBoundsJson.put("bottom", visibleBounds.bottom);
        json.put("visible_bounds", visibleBoundsJson);

        JSONArray children = new JSONArray();
        for (int i = 0; i < node.getChildCount(); i++) {
            AccessibilityNodeInfo child = node.getChild(i);
            if (child != null) {
                try {
                    children.put(buildNodeJson(child));
                } finally {
                    child.recycle();
                }
            }
        }
        json.put("children", children);
        return json;
    }

    /**
     * Compute the visible bounds of a node by walking up the parent chain
     * and intersecting bounds at each level. Handles overflow:hidden clipping.
     */
    private android.graphics.Rect getVisibleBounds(AccessibilityNodeInfo node) {
        android.graphics.Rect bounds = new android.graphics.Rect();
        node.getBoundsInScreen(bounds);
        AccessibilityNodeInfo parent = node.getParent();
        while (parent != null) {
            android.graphics.Rect parentBounds = new android.graphics.Rect();
            parent.getBoundsInScreen(parentBounds);
            if (!bounds.intersect(parentBounds)) {
                // Fully clipped — return zero-area rect
                parent.recycle();
                return new android.graphics.Rect();
            }
            AccessibilityNodeInfo next = parent.getParent();
            parent.recycle();
            parent = next;
        }
        return bounds;
    }

    /**
     * Parse cutout bounding rects from a mDisplayCutout line.
     * Format: boundingRect={Bounds=[Rect(0, 0 - 0, 0), Rect(480, 0 - 625, 136), ...]}
     * Returns non-zero-area rects as JSON: [{"x":480,"y":0,"width":145,"height":136}]
     */
    /**
     * Parse an InsetsSource frame: "frame=[LEFT,TOP][RIGHT,BOTTOM]"
     * Returns [left, top, right, bottom] or null on failure.
     */
    private int[] parseInsetsFrame(String line) {
        try {
            int frameIdx = line.indexOf("frame=[");
            if (frameIdx < 0) return null;
            String frameStr = line.substring(frameIdx + 7);
            String[] parts = frameStr.split("[\\[\\],]+");
            if (parts.length >= 4) {
                return new int[] {
                    Integer.parseInt(parts[0].trim()),
                    Integer.parseInt(parts[1].trim()),
                    Integer.parseInt(parts[2].trim()),
                    Integer.parseInt(parts[3].trim())
                };
            }
        } catch (Exception ignored) {}
        return null;
    }

    private JSONArray parseCutoutBounds(String line) {
        JSONArray result = new JSONArray();
        try {
            int boundsIdx = line.indexOf("Bounds=[");
            if (boundsIdx < 0) return result;
            String boundsStr = line.substring(boundsIdx + 8);
            int endIdx = boundsStr.indexOf("]}");
            if (endIdx > 0) boundsStr = boundsStr.substring(0, endIdx);
            // Match each Rect(left, top - right, bottom)
            java.util.regex.Pattern p = java.util.regex.Pattern.compile(
                    "Rect\\((\\d+),\\s*(\\d+)\\s*-\\s*(\\d+),\\s*(\\d+)\\)");
            java.util.regex.Matcher m = p.matcher(boundsStr);
            while (m.find()) {
                int left = Integer.parseInt(m.group(1));
                int top = Integer.parseInt(m.group(2));
                int right = Integer.parseInt(m.group(3));
                int bottom = Integer.parseInt(m.group(4));
                int w = right - left;
                int h = bottom - top;
                if (w > 0 && h > 0) {
                    JSONObject rect = new JSONObject();
                    rect.put("x", left);
                    rect.put("y", top);
                    rect.put("width", w);
                    rect.put("height", h);
                    result.put(rect);
                }
            }
        } catch (Exception ignored) {}
        return result;
    }

    /**
     * Parse rounded corners from a mRoundedCorners line.
     * Format: RoundedCorners{[RoundedCorner{position=TopLeft, radius=47, center=Point(47, 47)}, ...]}
     */
    private JSONArray parseRoundedCorners(String line) {
        JSONArray result = new JSONArray();
        try {
            java.util.regex.Pattern p = java.util.regex.Pattern.compile(
                    "position=(\\w+),\\s*radius=(\\d+),\\s*center=Point\\((\\d+),\\s*(\\d+)\\)");
            java.util.regex.Matcher m = p.matcher(line);
            while (m.find()) {
                String pos = m.group(1);
                int radius = Integer.parseInt(m.group(2));
                int cx = Integer.parseInt(m.group(3));
                int cy = Integer.parseInt(m.group(4));
                if (radius > 0) {
                    String posKey;
                    switch (pos) {
                        case "TopLeft": posKey = "top_left"; break;
                        case "TopRight": posKey = "top_right"; break;
                        case "BottomRight": posKey = "bottom_right"; break;
                        case "BottomLeft": posKey = "bottom_left"; break;
                        default: continue;
                    }
                    JSONObject corner = new JSONObject();
                    corner.put("position", posKey);
                    corner.put("radius", radius);
                    corner.put("center_x", cx);
                    corner.put("center_y", cy);
                    result.put(corner);
                }
            }
        } catch (Exception ignored) {}
        return result;
    }

    /**
     * Try to bind to the given port. If it fails and a PortAllocator is set,
     * re-register to get a new port and retry (up to 3 times).
     */
    private ServerSocket tryBind(int initialPort) throws IOException {
        int currentPort = initialPort;
        for (int attempt = 0; attempt < 3; attempt++) {
            try {
                return new ServerSocket(currentPort);
            } catch (IOException e) {
                if (portAllocator == null || attempt == 2) {
                    throw e; // No re-registration available or max retries
                }
                System.err.println("[golem] Port " + currentPort + " in use, re-registering...");
                try {
                    currentPort = portAllocator.allocatePort();
                    System.err.println("[golem] Re-registered on port " + currentPort);
                } catch (Exception re) {
                    throw new IOException("Re-registration failed: " + re.getMessage(), e);
                }
            }
        }
        throw new IOException("Failed to bind after 3 attempts");
    }

    private void executeShell(String command) throws IOException {
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(command);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[1024];
            while (is.read(buf) != -1) { /* drain */ }
        }
    }

    private String executeShellAndRead(String command) throws Exception {
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(command);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd);
             BufferedReader reader = new BufferedReader(new InputStreamReader(is))) {
            StringBuilder sb = new StringBuilder();
            String line;
            while ((line = reader.readLine()) != null) {
                sb.append(line).append("\n");
            }
            return sb.toString();
        }
    }

    private String escapeForInputText(String text) {
        StringBuilder sb = new StringBuilder();
        for (char c : text.toCharArray()) {
            switch (c) {
                case ' ':  sb.append("%s"); break;
                case '&':  sb.append("\\&"); break;
                case '<':  sb.append("\\<"); break;
                case '>':  sb.append("\\>"); break;
                case '|':  sb.append("\\|"); break;
                case ';':  sb.append("\\;"); break;
                case '(':  sb.append("\\("); break;
                case ')':  sb.append("\\)"); break;
                case '$':  sb.append("\\$"); break;
                case '`':  sb.append("\\`"); break;
                case '"':  sb.append("\\\""); break;
                case '\'': sb.append("\\'"); break;
                case '\\': sb.append("\\\\"); break;
                default:   sb.append(c); break;
            }
        }
        return sb.toString();
    }

    private void handleLaunch(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String packageName = req.optString("bundle_id", "");
        if (packageName.isEmpty()) {
            sendJson(out, 400, new JSONObject().put("error", "missing bundle_id"));
            return;
        }
        // Use am start with LAUNCHER category to bring app to foreground.
        // If already running, this activates it without restart. If not running, launches it.
        String cmd = "am start -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p " + packageName;
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(cmd);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[4096];
            while (is.read(buf) != -1) { /* drain */ }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleStop(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String packageName = req.optString("bundle_id", "");
        if (packageName.isEmpty()) {
            sendJson(out, 400, new JSONObject().put("error", "missing bundle_id"));
            return;
        }
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand("am force-stop " + packageName);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[4096];
            while (is.read(buf) != -1) { /* drain */ }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private Map<String, String> parseQueryParams(String fullPath) {
        Map<String, String> params = new HashMap<>();
        int qIdx = fullPath.indexOf('?');
        if (qIdx < 0) return params;
        String query = fullPath.substring(qIdx + 1);
        for (String pair : query.split("&")) {
            String[] kv = pair.split("=", 2);
            if (kv.length == 2) {
                params.put(kv[0], kv[1]);
            }
        }
        return params;
    }

    /**
     * Collect performance metrics for a package: file descriptors, disk usage, network bytes.
     * These require run-as or TrafficStats and can't be done from the host via adb shell.
     *
     * FDs and disk use `run-as <pkg>` which executes as the app's UID, granting access
     * to /proc/<pid>/fd and /data/data/<pkg> that the shell user (UID 2000) cannot read.
     */
    private void handlePerf(OutputStream out, Map<String, String> queryParams) throws Exception {
        String pkg = queryParams.get("package");
        if (pkg == null || pkg.isEmpty()) {
            sendJson(out, 400, new JSONObject().put("error", "missing ?package= query parameter"));
            return;
        }

        JSONObject result = new JSONObject();

        // Get PID — needed for FD counting
        int pid = -1;
        try {
            String pidOutput = executeShellAndRead("pidof " + pkg).trim();
            // pidof may return multiple PIDs (space-separated); take the first
            String firstPid = pidOutput.split("\\s+")[0];
            pid = Integer.parseInt(firstPid);
        } catch (Exception ignored) {}

        // File descriptors: run-as <pkg> ls /proc/<pid>/fd
        if (pid > 0) {
            try {
                String fdOutput = executeShellAndRead(
                    "run-as " + pkg + " ls /proc/" + pid + "/fd");
                int fdCount = 0;
                for (String line : fdOutput.split("\n")) {
                    if (!line.trim().isEmpty()) fdCount++;
                }
                if (fdCount > 0) {
                    result.put("file_descriptors", fdCount);
                } else {
                    result.put("file_descriptors", JSONObject.NULL);
                }
            } catch (Exception e) {
                result.put("file_descriptors", JSONObject.NULL);
            }
        } else {
            result.put("file_descriptors", JSONObject.NULL);
        }

        // Disk: run-as <pkg> du -sk /data/data/<pkg>
        try {
            String duOutput = executeShellAndRead(
                "run-as " + pkg + " du -sk /data/data/" + pkg);
            String firstField = duOutput.trim().split("\\s+")[0];
            long diskKb = Long.parseLong(firstField);
            result.put("disk_kb", diskKb);
        } catch (Exception e) {
            result.put("disk_kb", JSONObject.NULL);
        }

        // Network: TrafficStats by UID
        try {
            // Parse UID from dumpsys package (works on all Android versions)
            int uid = -1;
            String pkgInfo = executeShellAndRead("dumpsys package " + pkg);
            for (String line : pkgInfo.split("\n")) {
                String trimmed = line.trim();
                if (trimmed.startsWith("uid=")) {
                    uid = Integer.parseInt(trimmed.split("\\s+")[0].substring(4));
                    break;
                }
            }
            if (uid >= 0) {
                long rxBytes = android.net.TrafficStats.getUidRxBytes(uid);
                long txBytes = android.net.TrafficStats.getUidTxBytes(uid);
                result.put("net_rx_bytes", rxBytes >= 0 ? rxBytes : JSONObject.NULL);
                result.put("net_tx_bytes", txBytes >= 0 ? txBytes : JSONObject.NULL);
            } else {
                result.put("net_rx_bytes", JSONObject.NULL);
                result.put("net_tx_bytes", JSONObject.NULL);
            }
        } catch (Exception e) {
            result.put("net_rx_bytes", JSONObject.NULL);
            result.put("net_tx_bytes", JSONObject.NULL);
        }

        sendJson(out, 200, result);
    }

    private void sendJson(OutputStream out, int status, JSONObject json) throws IOException {
        String statusText;
        switch (status) {
            case 200: statusText = "OK"; break;
            case 400: statusText = "Bad Request"; break;
            case 404: statusText = "Not Found"; break;
            case 500: statusText = "Internal Server Error"; break;
            default:  statusText = "Unknown"; break;
        }
        byte[] body = json.toString().getBytes(StandardCharsets.UTF_8);
        String header = "HTTP/1.1 " + status + " " + statusText + "\r\n"
                + "Content-Type: application/json\r\n"
                + "Content-Length: " + body.length + "\r\n"
                + "Connection: close\r\n\r\n";
        out.write(header.getBytes(StandardCharsets.UTF_8));
        out.write(body);
        out.flush();
    }
}
