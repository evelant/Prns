import { useGlobalSearchParams, usePathname } from "expo-router";
import { ContactRuntimeProvider } from "@/native/contact-runtime-context";
import {
  DevelopmentMessagingScope,
  DevelopmentRuntimeProvider,
  routeConsumesDevelopmentSnapshot,
} from "@/native/development-runtime-context";
import { NamedIdentityProvider, useNamedIdentity } from "@/native/named-identity-context";
import { ShellLayout } from "@/ui/shell";

export default function ProductShellLayout() {
  const pathname = usePathname();
  const params = useGlobalSearchParams<{ owner?: string }>();

  return (
    <DevelopmentRuntimeProvider refreshActive={routeConsumesDevelopmentSnapshot(pathname)}>
      <NamedIdentityProvider
        pathname={pathname}
        {...(typeof params.owner === "string" ? { requestedOwner: params.owner } : {})}
      >
        <MessagingShell pathname={pathname} />
      </NamedIdentityProvider>
    </DevelopmentRuntimeProvider>
  );
}

function MessagingShell({ pathname }: { readonly pathname: string }) {
  const identity = useNamedIdentity();
  const messaging = /^\/(contacts|inbox)(?:\/|$)/.test(pathname);
  if (identity === null) return null;
  return (
    <DevelopmentMessagingScope
      projectIdentity={messaging}
      runtime={identity.runtime}
      identityHash={identity.identity?.reference.publicHash}
      ownerKey={identity.ownerKey}
      canPublish={identity.canPublish}
    >
      <ContactRuntimeProvider>
        <ShellLayout />
      </ContactRuntimeProvider>
    </DevelopmentMessagingScope>
  );
}
