import java.nio.file.*;
import org.junit.jupiter.api.*;
import static org.junit.jupiter.api.Assertions.*;

class JobProbeTest {
    @Test void executionContract() throws Exception {
        Files.writeString(Path.of("invocations.txt"), "run\n", StandardOpenOption.CREATE, StandardOpenOption.APPEND);
        Files.writeString(Path.of("started.txt"), Long.toString(ProcessHandle.current().pid()));
        assertEquals("test-only", System.getenv("SOMA_TEST_MODE"));
        assertNotEquals("runtime-only", System.getenv("SOMA_MODE"));
        String mode = System.getProperty("probe.mode", "pass");
        if (mode.equals("sleep")) Thread.sleep(12000);
        if (mode.equals("hang")) Thread.sleep(120000);
        if (mode.equals("mutate")) Files.writeString(Path.of("input.txt"), "changed during test");
        if (mode.equals("spam")) {
            String line = "bounded job log ñ ".repeat(400);
            for (int i = 0; i < 550; i++) System.out.println(line);
            System.out.println("JOB_SPAM_COMPLETE");
        }
        assertNotEquals("fail", mode, "Deliberate assertion failure");
    }
    @Test void anotherTest() { assertTrue(System.getProperty("java.version").startsWith("21.")); }
    @Disabled("Explicit fixture skip") @Test void omittedTest() { fail("Must be skipped"); }
}
