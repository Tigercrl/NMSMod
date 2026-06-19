#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>
#include <libgen.h>

int main(int argc, char *argv[]) {
    char *prog_path = argv[0];
    char *dir = dirname(prog_path);
    if (chdir(dir) != 0) {
        perror("chdir");
        return 1;
    }

    unsetenv("DYLD_INSERT_LIBRARIES");
    int ret = system("osascript -e 'tell application \"Terminal\"\n"
                     "    set newTab to do script \"nmsmod on-game-start\"\n"
                     "    repeat while busy of newTab\n"
                     "        delay 0.5\n"
                     "    end repeat\n"
                     "end tell'");
    if (ret == -1) {
        perror("system (prestart)");
        return 1;
    }

    pid_t pid = fork();
    if (pid == -1) {
        perror("fork");
        return 1;
    }

    if (pid == 0) {
        execv("./No Man's Sky", NULL);
        perror("execv");
        exit(1);
    } else {
        int status;
        waitpid(pid, &status, 0);

        ret = system("osascript -e 'tell application \"Terminal\" to do script \"nmsmod on-game-stop\"'");
        if (ret == -1) {
            perror("system (stop)");
            return 1;
        }

        if (WIFEXITED(status)) {
            return WEXITSTATUS(status);
        } else {
            return 1;
        }
    }
}
