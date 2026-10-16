import java.nio.file.*;
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
