// How long a hook starting a build may take: the build may have the whole workspace to compile.
export const BUILDING_MS = 1_800_000;

// A binary of the workspace's host package, built now so a test never meets an older one. Where it stands is cargo's
// to say, since a shared target directory moves it, so it is read off cargo's own report of the build.
export async function built(name: string): Promise<string> {
  const build = Bun.spawn(["cargo", "build", "-q", "-p", "mcp-host", "--bin", name, "--message-format=json"], {
    cwd: `${import.meta.dir}/..`,
    stdout: "pipe",
    stderr: "inherit",
  });
  const [said, exit] = await Promise.all([new Response(build.stdout).text(), build.exited]);

  if (exit !== 0) {
    throw new Error(`cargo could not build ${name}: it ended with ${exit}`);
  }

  for (const line of said.trim().split("\n")) {
    const message = JSON.parse(line) as { executable?: string | null };

    if (message.executable?.endsWith(`/${name}`)) {
      return message.executable;
    }
  }

  throw new Error(`cargo named no executable ${name}`);
}
