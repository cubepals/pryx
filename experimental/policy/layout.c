/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Values from Linux UAPI, emitted for the fixed coverage probe. */
#include <linux/bpf.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv)
{
    if (argc == 2 && strcmp(argv[1], "ifindex") == 0) {
        printf("%zu\n", offsetof(struct __sk_buff, ifindex));
        return 0;
    }
    if (argc != 1) return 1;
    printf("%zu %zu %zu %d %d\n", sizeof(struct __sk_buff),
           offsetof(struct __sk_buff, cb), sizeof(((struct __sk_buff *)0)->cb),
           BPF_FUNC_map_lookup_elem, BPF_FUNC_sk_storage_get);
    return 0;
}
