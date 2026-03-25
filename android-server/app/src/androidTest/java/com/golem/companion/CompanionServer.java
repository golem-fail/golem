package com.golem.companion;

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
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class CompanionServer {

    private static final int PORT = 8223;
    private final UiAutomation uiAutomation;

    public CompanionServer(UiAutomation uiAutomation) {
        this.uiAutomation = uiAutomation;
    }

    public void start() throws IOException {
        ServerSocket serverSocket = new ServerSocket(PORT);
        while (true) {
            Socket client = serverSocket.accept();
            new Thread(() -> {
                try {
                    handleClient(client);
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    try { client.close(); } catch (IOException ignored) {}
                }
            }).start();
        }
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
        Map<String, String> query = parseQuery(fullPath);

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
                    sendJson(out, 200, new JSONObject().put("status", "ok"));
                    break;
                case "/hierarchy":
                    handleHierarchy(out);
                    break;
                case "/tap":
                    handleTap(out, query);
                    break;
                case "/longpress":
                    handleLongPress(out, query);
                    break;
                case "/type":
                    handleType(out, query);
                    break;
                case "/backspace":
                    handleBackspace(out);
                    break;
                case "/swipe":
                    handleSwipe(out, query);
                    break;
                case "/screenshot":
                    handleScreenshot(out);
                    break;
                case "/hide-keyboard":
                    handleHideKeyboard(out);
                    break;
                case "/alert":
                    if ("POST".equals(method)) {
                        handleDismissAlert(out, body);
                    } else {
                        handleGetAlert(out);
                    }
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
            JSONObject json = buildNodeJson(root);
            sendJson(out, 200, json);
        } finally {
            root.recycle();
        }
    }

    private void handleTap(OutputStream out, Map<String, String> query) throws Exception {
        String x = query.get("x");
        String y = query.get("y");
        if (x == null || y == null) {
            sendJson(out, 400, new JSONObject().put("error", "x and y required"));
            return;
        }
        executeShell("input tap " + x + " " + y);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleLongPress(OutputStream out, Map<String, String> query) throws Exception {
        String x = query.get("x");
        String y = query.get("y");
        if (x == null || y == null) {
            sendJson(out, 400, new JSONObject().put("error", "x and y required"));
            return;
        }
        String duration = query.containsKey("duration") ? query.get("duration") : "1500";
        executeShell("input swipe " + x + " " + y + " " + x + " " + y + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleSwipe(OutputStream out, Map<String, String> query) throws Exception {
        String x1 = query.get("x1");
        String y1 = query.get("y1");
        String x2 = query.get("x2");
        String y2 = query.get("y2");
        if (x1 == null || y1 == null || x2 == null || y2 == null) {
            sendJson(out, 400, new JSONObject().put("error", "x1, y1, x2, y2 required"));
            return;
        }
        String duration = query.containsKey("duration") ? query.get("duration") : "300";
        executeShell("input swipe " + x1 + " " + y1 + " " + x2 + " " + y2 + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleType(OutputStream out, Map<String, String> query) throws Exception {
        String text = query.get("text");
        if (text == null) {
            sendJson(out, 400, new JSONObject().put("error", "text required"));
            return;
        }
        String escaped = escapeForInputText(text);
        executeShell("input text " + escaped);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleBackspace(OutputStream out) throws Exception {
        executeShell("input keyevent DEL");
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

    private void handleGetAlert(OutputStream out) throws Exception {
        List<AccessibilityWindowInfo> windows = uiAutomation.getWindows();
        for (AccessibilityWindowInfo window : windows) {
            if (window.getType() == AccessibilityWindowInfo.TYPE_SYSTEM) {
                AccessibilityNodeInfo root = window.getRoot();
                if (root != null) {
                    try {
                        JSONObject alert = new JSONObject();
                        alert.put("found", true);
                        alert.put("tree", buildNodeJson(root));
                        sendJson(out, 200, alert);
                        return;
                    } finally {
                        root.recycle();
                    }
                }
            }
        }
        sendJson(out, 200, new JSONObject().put("found", false));
    }

    private void handleDismissAlert(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body.isEmpty() ? "{}" : body);
        String action = req.optString("action", "dismiss");

        List<AccessibilityWindowInfo> windows = uiAutomation.getWindows();
        for (AccessibilityWindowInfo window : windows) {
            if (window.getType() == AccessibilityWindowInfo.TYPE_SYSTEM) {
                AccessibilityNodeInfo root = window.getRoot();
                if (root != null) {
                    try {
                        if (clickButtonByText(root, action)) {
                            sendJson(out, 200, new JSONObject().put("status", "ok"));
                            return;
                        }
                    } finally {
                        root.recycle();
                    }
                }
            }
        }
        sendJson(out, 404, new JSONObject().put("error", "alert button not found"));
    }

    private boolean clickButtonByText(AccessibilityNodeInfo node, String text) {
        if (node == null) return false;
        CharSequence nodeText = node.getText();
        if (nodeText != null && nodeText.toString().equalsIgnoreCase(text)) {
            node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            return true;
        }
        for (int i = 0; i < node.getChildCount(); i++) {
            AccessibilityNodeInfo child = node.getChild(i);
            if (child != null) {
                try {
                    if (clickButtonByText(child, text)) return true;
                } finally {
                    child.recycle();
                }
            }
        }
        return false;
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

    private void executeShell(String command) throws IOException {
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(command);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[1024];
            while (is.read(buf) != -1) { /* drain */ }
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

    private Map<String, String> parseQuery(String fullPath) {
        Map<String, String> query = new HashMap<>();
        int idx = fullPath.indexOf('?');
        if (idx < 0) return query;
        String qs = fullPath.substring(idx + 1);
        for (String param : qs.split("&")) {
            String[] kv = param.split("=", 2);
            if (kv.length == 2) {
                try {
                    query.put(URLDecoder.decode(kv[0], "UTF-8"),
                              URLDecoder.decode(kv[1], "UTF-8"));
                } catch (Exception ignored) {}
            }
        }
        return query;
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
