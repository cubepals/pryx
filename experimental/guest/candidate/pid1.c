/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Linux invokes the small static main in vm-init mode; the maintained supervisor
 * validates the actual root PID, bootstrap image and CID and owns all proofs. */
#include <string.h>

extern int pryx_lab_supervise(void);
#ifdef LAB_ACTIVATION
extern int pryx_lab_vm_agent(void);
#endif

int main(int argc, char **argv) {
#ifdef LAB_ACTIVATION
    if (argc > 2 && strcmp(argv[1], "vm-agent") == 0) {
        return pryx_lab_vm_agent();
    }
#endif
    if (argc != 2 || strcmp(argv[1], "vm-init") != 0) {
        return 64;
    }
    return pryx_lab_supervise();
}
