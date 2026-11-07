use strict;
use warnings;
use Config;
use Fcntl ':mode';
use Math::BigInt;
use POSIX 'uname';

sub geometry {
    my ($io) = @_;
    my $data = "\0" x 112;
    $io->(0x80705864, \$data);
    my ($blocksize, $agblocks, $imaxpct) = map { unpack('L', substr($data, $_, 4)) } (0, 8, 28);
    my ($blocks, $rtblocks, $rtextents, $logstart) = unpack('Q4', substr($data, 32, 32));
    die "invalid XFS geometry\n" if $blocksize < 512 || ($blocksize & ($blocksize - 1)) || $agblocks < 64 || !$blocks || $imaxpct > 100;
    die "XFS realtime or external-log layout unsupported\n" if $rtblocks || $rtextents || !$logstart;
    return ($blocksize, $imaxpct, $blocks, $agblocks);
}

sub execute_xfs {
    my ($mode, $bytes, $expected, $identity, $io) = @_;
    die "invalid helper arguments\n" unless $mode =~ /\A(?:preflight|check|grow)\z/ && $bytes =~ /\A[0-9]+\z/ && $expected =~ /\A[0-9]+\z/;
    my ($dev, $is_dir) = $identity->();
    die "mount descriptor identity mismatch\n" unless $is_dir && "$dev" eq "$expected";
    my ($blocksize, $imaxpct, $before, $agblocks) = geometry($io);
    return 0 if $mode eq 'preflight';
    my $target = Math::BigInt->new($bytes)->bdiv($blocksize);
    my ($groups, $remainder) = $target->copy()->bdiv($agblocks);
    die "XFS allocation group limit exceeded\n" if $groups->bcmp('4294967295') >= 0;
    $target->bsub($remainder) if !$remainder->is_zero() && $remainder->bcmp(64) < 0;
    die "refusing XFS shrink\n" if $target->bcmp("$before") < 0;
    return $target->bcmp("$before") > 0 ? 1 : 0 if $mode eq 'check' || $target->bcmp("$before") == 0;
    my $request = pack('QLL', $target->bstr(), $imaxpct, 0);
    die "invalid XFS ioctl ABI\n" unless length($request) == 16;
    $io->(0x4010586e, \$request);
    my ($after_size, $after_imax, $after, $after_ag) = geometry($io);
    die "XFS growth postcondition failed\n" if $after_size != $blocksize || $after_imax != $imaxpct || $after_ag != $agblocks || $target->bcmp("$after") != 0;
    return 1;
}

sub main_xfs {
    my ($mode, $bytes, $expected) = @_;
    my @uname = uname();
    die "unsupported XFS ioctl ABI\n" unless $^O eq 'linux' && ($uname[4] eq 'x86_64' || $uname[4] eq 'aarch64') && $Config{ivsize} == 8 && $Config{uvsize} == 8 && length(pack('Q', 0)) == 8 && length(pack('L', 0)) == 4;
    open my $mount, '<&=3' or die "cannot open mount descriptor: $!\n";
    my $changed = execute_xfs($mode, $bytes, $expected,
        sub { my @st = stat($mount); die "cannot stat mount descriptor: $!\n" unless @st; return ($st[0], S_ISDIR($st[2])); },
        sub { my ($request, $buffer) = @_; ioctl($mount, $request, $$buffer) or die "XFS ioctl failed: $!\n"; });
    print $changed ? "{\"changed\":true}\n" : "{\"changed\":false}\n";
}
