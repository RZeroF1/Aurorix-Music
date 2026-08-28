using System;
using Microsoft.Win32.SafeHandles;

namespace Aurorix.Windows.Interop;

internal sealed class AurorixClientSafeHandle : SafeHandleZeroOrMinusOneIsInvalid
{
    internal AurorixClientSafeHandle(IntPtr handle)
        : base(ownsHandle: true)
    {
        SetHandle(handle);
    }

    protected override bool ReleaseHandle() =>
        AurorixNativeMethods.aurorix_client_release_v1(handle) is
            AurorixNativeMethods.StatusOk or AurorixNativeMethods.StatusShutdownIncomplete;
}

internal sealed class AurorixOperationSafeHandle : SafeHandleZeroOrMinusOneIsInvalid
{
    internal AurorixOperationSafeHandle(IntPtr handle)
        : base(ownsHandle: true)
    {
        SetHandle(handle);
    }

    protected override bool ReleaseHandle() =>
        AurorixNativeMethods.aurorix_operation_release_v1(handle) == AurorixNativeMethods.StatusOk;
}

internal sealed class AurorixSubscriptionSafeHandle : SafeHandleZeroOrMinusOneIsInvalid
{
    internal AurorixSubscriptionSafeHandle(IntPtr handle)
        : base(ownsHandle: true)
    {
        SetHandle(handle);
    }

    protected override bool ReleaseHandle() =>
        AurorixNativeMethods.aurorix_subscription_release_v1(handle) == AurorixNativeMethods.StatusOk;
}
