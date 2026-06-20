import Foundation
import Testing

/// Unit tests for `HTTPResponse`'s pure value builders. These run in a logic
/// unit-test bundle — no simulator UI automation involved.
struct HTTPResponseTests {
    @Test func jsonSetsStatusContentTypeAndSortsKeys() {
        let r = HTTPResponse.json(["b": 2, "a": 1])
        #expect(r.statusCode == 200)
        #expect(r.statusText == "OK")
        #expect(r.contentType == "application/json")
        // `.sortedKeys` → "a" emitted before "b".
        #expect(String(data: r.body, encoding: .utf8) == #"{"a":1,"b":2}"#)
    }

    @Test func jsonHonorsExplicitStatus() {
        let r = HTTPResponse.json(["ok": true], status: 404)
        #expect(r.statusCode == 404)
        #expect(r.statusText == "Not Found")
    }

    @Test func errorWrapsMessageAsJSON() throws {
        let r = HTTPResponse.error("boom", status: 400)
        #expect(r.statusCode == 400)
        #expect(r.statusText == "Bad Request")
        #expect(r.contentType == "application/json")
        let obj = try JSONSerialization.jsonObject(with: r.body) as? [String: String]
        #expect(obj?["error"] == "boom")
    }

    @Test func pngUsesImageContentType() {
        let bytes = Data([0x89, 0x50, 0x4E, 0x47])
        let r = HTTPResponse.png(bytes)
        #expect(r.statusCode == 200)
        #expect(r.statusText == "OK")
        #expect(r.contentType == "image/png")
        #expect(r.body == bytes)
    }

    @Test func jsonDataPassesBytesThrough() {
        let raw = Data(#"{"x":1}"#.utf8)
        let r = HTTPResponse.jsonData(raw, status: 500)
        #expect(r.statusCode == 500)
        #expect(r.statusText == "Internal Server Error")
        #expect(r.body == raw)
    }

    @Test(arguments: [
        (code: 200, text: "OK"),
        (code: 400, text: "Bad Request"),
        (code: 404, text: "Not Found"),
        (code: 500, text: "Internal Server Error"),
        (code: 418, text: "Unknown"),
    ])
    func statusTextMapping(_ c: (code: Int, text: String)) {
        // `statusText(for:)` is private; exercise it through a builder.
        #expect(HTTPResponse.jsonData(Data(), status: c.code).statusText == c.text)
    }

    // MARK: - serialized() wire bytes (single-buffer write)

    @Test func serializedContainsStatusLineHeadersBlankLineThenBody() {
        let r = HTTPResponse.jsonData(Data(#"{"a":1}"#.utf8), status: 200)
        let wire = String(data: r.serialized(), encoding: .utf8)!
        #expect(wire.hasPrefix("HTTP/1.1 200 OK\r\n"))
        #expect(wire.contains("Content-Type: application/json\r\n"))
        #expect(wire.contains("Content-Length: 7\r\n"))   // {"a":1} = 7 bytes
        #expect(wire.contains("Connection: close\r\n"))
        // Headers terminated by a blank line, then the body verbatim.
        #expect(wire.hasSuffix("\r\n\r\n{\"a\":1}"))
    }

    @Test func serializedContentLengthMatchesBodyByteCount() {
        // Multi-byte UTF-8 body: Content-Length is BYTES, not characters.
        let body = Data("café 世界".utf8)
        let r = HTTPResponse.jsonData(body, status: 200)
        let wire = r.serialized()
        #expect(wire.range(of: Data("Content-Length: \(body.count)\r\n".utf8)) != nil)
        // The body bytes appear unmodified at the tail.
        #expect(wire.suffix(body.count) == body)
    }

    @Test func serializedEmptyBodyIsHeaderOnlyWithZeroLength() {
        let r = HTTPResponse.jsonData(Data(), status: 404)
        let wire = String(data: r.serialized(), encoding: .utf8)!
        #expect(wire.contains("HTTP/1.1 404 Not Found\r\n"))
        #expect(wire.contains("Content-Length: 0\r\n"))
        #expect(wire.hasSuffix("\r\n\r\n"))
    }

    @Test func serializedPreservesBinaryBody() {
        // PNG magic bytes must survive serialization intact (no UTF-8 coercion).
        let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        let wire = HTTPResponse.png(png).serialized()
        #expect(wire.suffix(png.count) == png)
        #expect(wire.range(of: Data("Content-Type: image/png\r\n".utf8)) != nil)
    }
}
