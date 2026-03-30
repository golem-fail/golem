import Foundation

/// A lightweight HTTP response value type.
struct HTTPResponse {
    let statusCode: Int
    let statusText: String
    let contentType: String
    let body: Data

    static func json(_ object: Any, status: Int = 200) -> HTTPResponse {
        let data = (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])) ?? Data()
        return HTTPResponse(
            statusCode: status,
            statusText: statusText(for: status),
            contentType: "application/json",
            body: data
        )
    }

    static func jsonData(_ data: Data, status: Int = 200) -> HTTPResponse {
        return HTTPResponse(
            statusCode: status,
            statusText: statusText(for: status),
            contentType: "application/json",
            body: data
        )
    }

    static func png(_ data: Data) -> HTTPResponse {
        return HTTPResponse(
            statusCode: 200,
            statusText: "OK",
            contentType: "image/png",
            body: data
        )
    }

    static func error(_ message: String, status: Int = 500) -> HTTPResponse {
        let obj: [String: Any] = ["error": message]
        let data = (try? JSONSerialization.data(withJSONObject: obj, options: [])) ?? Data()
        return HTTPResponse(
            statusCode: status,
            statusText: statusText(for: status),
            contentType: "application/json",
            body: data
        )
    }

    private static func statusText(for code: Int) -> String {
        switch code {
        case 200: return "OK"
        case 400: return "Bad Request"
        case 404: return "Not Found"
        case 500: return "Internal Server Error"
        default: return "Unknown"
        }
    }
}

/// Type alias for the request handler closure.
typealias RequestHandler = (_ method: String, _ path: String, _ body: Data?) -> HTTPResponse

/// A minimal BSD-socket HTTP/1.1 server that runs inside an XCUITest process.
final class HTTPServer {
    private let port: UInt16
    private let handler: RequestHandler
    private var listenSocket: Int32 = -1
    private var running = false
    private var acceptThread: Thread?

    /// Inactivity timeout — server exits after this duration with no requests.
    private let inactivityTimeout: TimeInterval = 5 * 60 * 60 // 5 hours
    private var inactivityTimer: DispatchSourceTimer?

    init(port: UInt16, handler: @escaping RequestHandler) {
        self.port = port
        self.handler = handler
    }

    func start() throws {
        listenSocket = socket(AF_INET, SOCK_STREAM, 0)
        guard listenSocket >= 0 else {
            throw ServerError.socketCreationFailed
        }

        var reuse: Int32 = 1
        setsockopt(listenSocket, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = INADDR_ANY

        let bindResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                bind(listenSocket, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            close(listenSocket)
            throw ServerError.bindFailed(port: port)
        }

        guard listen(listenSocket, 8) == 0 else {
            close(listenSocket)
            throw ServerError.listenFailed
        }

        running = true
        startInactivityTimer()

        let thread = Thread { [weak self] in
            self?.acceptLoop()
        }
        thread.name = "GolemHTTPServer"
        thread.qualityOfService = .userInitiated
        thread.start()
        acceptThread = thread
    }

    func stop() {
        running = false
        if listenSocket >= 0 {
            close(listenSocket)
            listenSocket = -1
        }
    }

    // MARK: - Inactivity timer

    private func startInactivityTimer() {
        let timer = DispatchSource.makeTimerSource(queue: .global())
        timer.schedule(deadline: .now() + inactivityTimeout)
        timer.setEventHandler {
            NSLog("Golem companion: shutting down after %.0f hours of inactivity", self.inactivityTimeout / 3600)
            exit(0)
        }
        timer.resume()
        inactivityTimer = timer
    }

    private func resetInactivityTimer() {
        inactivityTimer?.cancel()
        startInactivityTimer()
    }

    // MARK: - Private

    private func acceptLoop() {
        while running {
            var clientAddr = sockaddr_in()
            var addrLen = socklen_t(MemoryLayout<sockaddr_in>.size)
            let clientSocket = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                    accept(listenSocket, sockaddrPtr, &addrLen)
                }
            }
            guard clientSocket >= 0 else {
                continue
            }
            let handler = self.handler
            Thread.detachNewThread {
                self.handleConnection(socket: clientSocket, handler: handler)
            }
        }
    }

    private func handleConnection(socket clientSocket: Int32, handler: RequestHandler) {
        defer { close(clientSocket) }

        guard let (method, fullPath, headers, headerData) = readRequestHead(socket: clientSocket) else {
            return
        }

        // Read body if Content-Length is present.
        var body: Data?
        if let clValue = headers["content-length"], let contentLength = Int(clValue), contentLength > 0 {
            body = readBody(socket: clientSocket, headerData: headerData, contentLength: contentLength)
        }

        resetInactivityTimer()
        let response = handler(method, fullPath, body)
        writeResponse(socket: clientSocket, response: response)
    }

    private func readRequestHead(socket: Int32) -> (method: String, path: String, headers: [String: String], allData: Data)? {
        var buffer = Data()
        let chunkSize = 4096
        let chunk = UnsafeMutablePointer<UInt8>.allocate(capacity: chunkSize)
        defer { chunk.deallocate() }

        // Read until we find \r\n\r\n (end of headers).
        while true {
            let bytesRead = recv(socket, chunk, chunkSize, 0)
            guard bytesRead > 0 else { return nil }
            buffer.append(chunk, count: bytesRead)

            if let headerEnd = buffer.range(of: Data([0x0D, 0x0A, 0x0D, 0x0A])) {
                let headerBytes = buffer[buffer.startIndex..<headerEnd.lowerBound]
                guard let headerString = String(data: headerBytes, encoding: .utf8) else { return nil }

                let lines = headerString.components(separatedBy: "\r\n")
                guard let requestLine = lines.first else { return nil }
                let parts = requestLine.split(separator: " ", maxSplits: 2)
                guard parts.count >= 2 else { return nil }

                let method = String(parts[0])
                let path = String(parts[1])

                var headers: [String: String] = [:]
                for line in lines.dropFirst() {
                    if let colonIndex = line.firstIndex(of: ":") {
                        let key = line[line.startIndex..<colonIndex].trimmingCharacters(in: .whitespaces).lowercased()
                        let value = line[line.index(after: colonIndex)...].trimmingCharacters(in: .whitespaces)
                        headers[key] = value
                    }
                }

                return (method, path, headers, buffer)
            }

            if buffer.count > 65536 {
                return nil
            }
        }
    }

    private func readBody(socket: Int32, headerData: Data, contentLength: Int) -> Data {
        // Find where the body starts (after \r\n\r\n).
        guard let headerEnd = headerData.range(of: Data([0x0D, 0x0A, 0x0D, 0x0A])) else {
            return Data()
        }
        var body = headerData[headerEnd.upperBound...]

        let chunkSize = 4096
        let chunk = UnsafeMutablePointer<UInt8>.allocate(capacity: chunkSize)
        defer { chunk.deallocate() }

        while body.count < contentLength {
            let bytesRead = recv(socket, chunk, min(chunkSize, contentLength - body.count), 0)
            guard bytesRead > 0 else { break }
            body.append(chunk, count: bytesRead)
        }
        return Data(body)
    }

    private func writeResponse(socket: Int32, response: HTTPResponse) {
        var header = "HTTP/1.1 \(response.statusCode) \(response.statusText)\r\n"
        header += "Content-Type: \(response.contentType)\r\n"
        header += "Content-Length: \(response.body.count)\r\n"
        header += "Connection: close\r\n"
        header += "\r\n"

        let headerData = Data(header.utf8)
        headerData.withUnsafeBytes { ptr in
            _ = send(socket, ptr.baseAddress!, headerData.count, 0)
        }
        response.body.withUnsafeBytes { ptr in
            if response.body.count > 0 {
                _ = send(socket, ptr.baseAddress!, response.body.count, 0)
            }
        }
    }

    enum ServerError: Error, LocalizedError {
        case socketCreationFailed
        case bindFailed(port: UInt16)
        case listenFailed

        var errorDescription: String? {
            switch self {
            case .socketCreationFailed: return "Failed to create socket"
            case .bindFailed(let port): return "Failed to bind to port \(port)"
            case .listenFailed: return "Failed to listen on socket"
            }
        }
    }
}
