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

public class CompanionServer {

    private static final int DEFAULT_PORT = 8223;
    /** Inactivity timeout — server exits after this duration with no requests. */
    private static final long INACTIVITY_TIMEOUT_MS = 5 * 60 * 60 * 1000L; // 5 hours
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
        String path = parts[1].split("\\?")[0];

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
                        .put("version", "0.4.2")
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
                default:
                    sendJson(out, 404, new JSONObject().put("error", "not found"));
                    break;
            }
        } catch (Exception e) {
            sendJson(out, 500, new JSONObject().put("error", e.getMessage()));
        }
    }

    private void handleHierarchy(OutputStream out) throws Exception {
        AccessibilityNodeInfo root = uiAutomation.getRootInActiveWindow();
        if (root == null) {
            sendJson(out, 500, new JSONObject().put("error", "no active window"));
            return;
        }
        try {
            // Parse dumpsys window for keyboard, safe area, cutouts, and corners.
            int keyboardHeight = 0;
            int safeAreaTop = 0;
            int safeAreaBottom = 0;
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
        Bitmap bitmap = uiAutomation.takeScreenshot();
        if (bitmap == null) {
            sendJson(out, 500, new JSONObject().put("error", "screenshot failed"));
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
        executeShell("input keyevent KEYCODE_BACK");
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
        // Use monkey to launch — more reliable than am start via executeShellCommand
        String cmd = "monkey -p " + packageName + " -c android.intent.category.LAUNCHER 1";
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
