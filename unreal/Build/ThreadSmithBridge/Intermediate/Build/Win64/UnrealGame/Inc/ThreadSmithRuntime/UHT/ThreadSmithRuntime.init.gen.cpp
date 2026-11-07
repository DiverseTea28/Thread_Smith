// Copyright Epic Games, Inc. All Rights Reserved.
/*===========================================================================
	Generated code exported from UnrealHeaderTool.
	DO NOT modify this manually! Edit the corresponding .h files instead!
===========================================================================*/

#include "UObject/GeneratedCppIncludes.h"
PRAGMA_DISABLE_DEPRECATION_WARNINGS
void EmptyLinkFunctionForGeneratedCodeThreadSmithRuntime_init() {}
static_assert(!UE_WITH_CONSTINIT_UOBJECT, "This generated code can only be compiled with !UE_WITH_CONSTINIT_OBJECT");
	static FPackageRegistrationInfo Z_Registration_Info_UPackage__Script_ThreadSmithRuntime;
	FORCENOINLINE UPackage* Z_Construct_UPackage__Script_ThreadSmithRuntime(ETypeConstructPhase)
	{
		if (!Z_Registration_Info_UPackage__Script_ThreadSmithRuntime.OuterSingleton)
		{
		static const UECodeGen_Private::FPackageParams PackageParams = {
			"/Script/ThreadSmithRuntime",
			nullptr,
			0,
			PKG_CompiledIn | 0x00000000,
			0xBBDCADFE,
			0xB1D9C514,
			METADATA_PARAMS(0, nullptr)
		};
		UECodeGen_Private::ConstructUPackage(Z_Registration_Info_UPackage__Script_ThreadSmithRuntime.OuterSingleton, PackageParams);
	}
	return Z_Registration_Info_UPackage__Script_ThreadSmithRuntime.OuterSingleton;
}
static FRegisterCompiledInInfo Z_CompiledInDeferPackage_UPackage__Script_ThreadSmithRuntime(Z_Construct_UPackage__Script_ThreadSmithRuntime, TEXT("/Script/ThreadSmithRuntime"), Z_Registration_Info_UPackage__Script_ThreadSmithRuntime, CONSTRUCT_RELOAD_VERSION_INFO(FPackageReloadVersionInfo, 0xBBDCADFE, 0xB1D9C514));
PRAGMA_ENABLE_DEPRECATION_WARNINGS
