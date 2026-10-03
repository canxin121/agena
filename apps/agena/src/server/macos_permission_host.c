// The installed, ad-hoc signed copy of this supervisor must remain byte-for-byte
// unchanged across ordinary server upgrades. TCC follows the responsible app
// through posix_spawn; exec would discard the identity we need to preserve.
#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;
static volatile sig_atomic_t child_pid = 0;

static void forward_signal(int signal_number) {
    int saved_errno = errno;
    if (child_pid > 0) {
        kill((pid_t)child_pid, signal_number);
    }
    errno = saved_errno;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--agena-host-protocol") == 0) {
        puts("1");
        return 0;
    }
    if (argc < 2 || argv[1][0] != '/') {
        fprintf(stderr, "Agena permission host requires an absolute server executable.\n"
                        "Run `agena server permissions` for macOS setup.\n");
        return 64;
    }

    // Block shutdown signals until both the child PID and handlers exist, and
    // restore the caller's mask/default handlers inside the spawned server.
    sigset_t signals, previous_mask;
    sigemptyset(&signals);
    sigaddset(&signals, SIGTERM);
    sigaddset(&signals, SIGINT);
    sigaddset(&signals, SIGHUP);
    if (sigprocmask(SIG_BLOCK, &signals, &previous_mask) != 0) {
        perror("Agena permission host: sigprocmask");
        return 71;
    }
    struct sigaction handler = {0};
    handler.sa_handler = forward_signal;
    sigemptyset(&handler.sa_mask);
    if (sigaction(SIGTERM, &handler, NULL) != 0 ||
        sigaction(SIGINT, &handler, NULL) != 0 ||
        sigaction(SIGHUP, &handler, NULL) != 0) {
        perror("Agena permission host: sigaction");
        return 71;
    }

    posix_spawnattr_t attributes;
    int error = posix_spawnattr_init(&attributes);
    if (error != 0) {
        fprintf(stderr, "Agena permission host: %s\n", strerror(error));
        return 71;
    }
    if ((error = posix_spawnattr_setsigmask(&attributes, &previous_mask)) == 0 &&
        (error = posix_spawnattr_setsigdefault(&attributes, &signals)) == 0 &&
        (error = posix_spawnattr_setflags(&attributes,
                     POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_SETSIGDEF)) == 0) {
        pid_t pid;
        error = posix_spawn(&pid, argv[1], NULL, &attributes, &argv[1], environ);
        if (error == 0) {
            child_pid = pid;
        }
    }
    posix_spawnattr_destroy(&attributes);
    if (error != 0) {
        fprintf(stderr, "Agena permission host: cannot start server: %s\n", strerror(error));
        return 71;
    }
    if (sigprocmask(SIG_SETMASK, &previous_mask, NULL) != 0) {
        kill((pid_t)child_pid, SIGKILL);
        perror("Agena permission host: restore signal mask");
        return 71;
    }

    int status;
    while (waitpid((pid_t)child_pid, &status, 0) < 0) {
        if (errno != EINTR) {
            perror("Agena permission host: waitpid");
            return 71;
        }
    }
    child_pid = 0;
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
