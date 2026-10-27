# No host process executes the orphan fixture. All signals stay in this VM.
{ pkgs }:
pkgs.testers.runNixOSTest {
  name = "script-signal-ownership";
  qemu.forceAccel = true;
  nodes.machine = {
    networking.hostName = "script-signal-guest";
    virtualisation.cores = 2;
    virtualisation.memorySize = 1024;
    users.users.tester = { isNormalUser = true; uid = 1000; linger = true; };
    environment.systemPackages = [ pkgs.python3 ];
  };
  testScript = ''
    import shlex
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("default.target", "tester")
    assert machine.succeed("systemd-detect-virt").strip() == "kvm"
    manager = machine.succeed("systemctl show user@1000.service -p MainPID --value").strip()
    assert int(manager) > 1
    machine.succeed("install -d -o tester /tmp/signal-proof")

    def user(command):
        return "su tester -s /bin/sh -c " + shlex.quote("XDG_RUNTIME_DIR=/run/user/1000 " + command)

    fixture = "python3 ${../scripts}/script-orphan-guest-test"
    # Same service topology as a test launched by the fleet user manager.
    # ExitType=cgroup keeps the observation service until its orphan exits.
    machine.succeed(user("systemd-run --user --wait --pipe --collect --quiet -p ExitType=cgroup -- "
                         + fixture + " observe /tmp/signal-proof/control " + manager))
    control = machine.succeed("cat /tmp/signal-proof/control").split()
    assert control[1] == manager, control

    machine.fail(user("systemd-run --user --wait --pipe --collect --quiet -p ExitType=cgroup -- "
                      + "python3 ${../scripts}/st3_test_process.py -- "
                      + fixture + " signal /tmp/signal-proof/guarded " + manager))
    guarded = machine.succeed("cat /tmp/signal-proof/guarded").split()
    assert guarded[1] != manager, guarded
    assert guarded[1] != control[0], guarded
    assert machine.succeed("systemctl show user@1000.service -p MainPID --value").strip() == manager
    machine.succeed(user("systemctl --user is-active default.target"))
    for pid in guarded:
        machine.fail("test -e /proc/" + pid)
    # Prove the shell entrypoint uses the same guardian, and ordinary test
    # results still pass through; installer signal fixtures are pipe-only.
    machine.succeed(user("systemd-run --user --wait --pipe --collect --quiet -- "
                         + "python3 ${../scripts}/script-process-test"))
  '';
}
