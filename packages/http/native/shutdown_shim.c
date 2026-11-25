/* Signal-to-flag bridge for Yashy services.
 *
 * The handler only sets a sig_atomic_t flag; no allocation, locking or runtime
 * call happens inside it. The program polls `yashy_shutdown_pending` at a
 * point where the service lifecycle can drain safely.
 */
#include <signal.h>

static volatile sig_atomic_t yashy_shutdown_flag = 0;

static void yashy_shutdown_handler(int signum) {
    (void)signum;
    yashy_shutdown_flag = 1;
}

int yashy_shutdown_install(void) {
    signal(SIGINT, yashy_shutdown_handler);
    signal(SIGTERM, yashy_shutdown_handler);
    return 1;
}

int yashy_shutdown_pending(void) {
    return yashy_shutdown_flag ? 1 : 0;
}
