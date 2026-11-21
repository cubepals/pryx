/* SPDX-License-Identifier: FSL-1.1-ALv2 */
/* Values from Linux UAPI, emitted for the fixed coverage probe. */
#include <linux/bpf.h>
#include <stddef.h>
#include <stdio.h>
int main(void)
{
    printf("%zu %zu %zu %d %d\n", sizeof(struct __sk_buff),
           offsetof(struct __sk_buff, cb), sizeof(((struct __sk_buff *)0)->cb),
           BPF_FUNC_map_lookup_elem, BPF_FUNC_sk_storage_get);
    return 0;
}
