import java.nio.file.*;
import java.net.InetSocketAddress;
import com.sun.net.httpserver.HttpServer;
public class Probe {
    public static void main(String[] args) throws Exception {
        String mode = args.length == 0 ? "normal" : args[0];
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                Files.writeString(Path.of("stopped-" + mode + ".txt"), "shutdown-hook");
                if (mode.equals("hang")) Thread.sleep(60000);
            } catch (Exception ignored) { }
        }));
        System.out.println("PARENT_PID=" + ProcessHandle.current().pid());
        System.out.println("MODE=" + System.getenv("SOMA_MODE"));
        System.out.println("UNICODE=áéíóú ñ ✓");
        System.out.println("CWD=" + Path.of("").toAbsolutePath());
        if (Files.exists(Path.of("marker.txt"))) System.out.println("MARKER=" + Files.readString(Path.of("marker.txt")).strip());
        if (mode.equals("exit")) return;
        if (mode.equals("http") || mode.equals("down") || mode.equals("http-wildcard")) {
            long readyAt = System.nanoTime() + Long.parseLong(args[2]) * 1_000_000L;
            int port = Integer.parseInt(args[1]);
            HttpServer server = HttpServer.create(mode.equals("http-wildcard") ? new InetSocketAddress(port) : new InetSocketAddress("127.0.0.1", port), 0);
            server.createContext("/health", exchange -> {
                boolean ready = !mode.equals("down") && System.nanoTime() >= readyAt;
                byte[] body = ("{\"status\":\"" + (ready ? "UP" : "DOWN") + "\"}").getBytes(java.nio.charset.StandardCharsets.UTF_8);
                exchange.getResponseHeaders().set("Content-Type", "application/json");
                exchange.sendResponseHeaders(200, body.length);
                exchange.getResponseBody().write(body);
                exchange.close();
            });
            server.start();
            System.out.println("HTTP_STARTED");
        }
        if (mode.equals("spam")) {
            String line = "ñ✓".repeat(1800) + "\n";
            for (int i = 0; i < 6000; i++) System.out.print(line);
            System.out.println("SPAM_COMPLETE");
            return;
        }
        if (mode.equals("paced")) {
            String line = "Sustained log áéíóú ñ ✓ ".repeat(300) + "\n";
            long deadline = System.nanoTime() + 40_000_000_000L;
            while (System.nanoTime() < deadline) {
                System.out.print(line);
                Thread.sleep(1);
            }
            System.out.println("PACED_COMPLETE");
            return;
        }
        if (mode.equals("normal") || mode.equals("hang")) {
            Process child = new ProcessBuilder(Path.of(System.getProperty("java.home"), "bin", "java.exe").toString(),
                "-Xmx32m", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8", "-cp", System.getProperty("java.class.path"), "Probe", "child").inheritIO().start();
            System.out.println("CHILD_PID=" + child.pid());
        }
        System.out.flush();
        while (true) Thread.sleep(100);
    }
}
