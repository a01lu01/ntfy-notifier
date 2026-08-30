import com.android.build.api.instrumentation.AsmClassVisitorFactory
import com.android.build.api.instrumentation.ClassContext
import com.android.build.api.instrumentation.ClassData
import com.android.build.api.instrumentation.InstrumentationParameters
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.MethodVisitor
import org.objectweb.asm.Opcodes

private val jacksonApi24ParameterCountCalls = mapOf(
    "com.fasterxml.jackson.databind.deser.BeanDeserializerBase" to 1,
    "com.fasterxml.jackson.databind.introspect.AnnotatedConstructor" to 2,
    "com.fasterxml.jackson.databind.introspect.AnnotatedCreatorCollector" to 1,
    "com.fasterxml.jackson.databind.introspect.AnnotatedMethod" to 1,
    "com.fasterxml.jackson.databind.introspect.AnnotatedMethodCollector" to 1,
    "com.fasterxml.jackson.databind.util.ClassUtil\$Ctor" to 1,
)

private const val jacksonExceptionUtil = "com.fasterxml.jackson.databind.util.ExceptionUtil"
private const val bootstrapMethodError = "java/lang/BootstrapMethodError"
private const val api24CompatHelper = "app/ntfy/notifier/JacksonApi24Compat"
private val api24UnsafeReflectionOwners =
    setOf("java/lang/reflect/Constructor", "java/lang/reflect/Method")

/**
 * Rewrites the API 26-only JDK references used by Tauri's Jackson 2.15.3 dependency.
 *
 * Exact per-class rewrite counts reject instruction drift. The app build separately rejects a
 * resolved Jackson version other than the one represented by this allowlist.
 */
abstract class JacksonApi24CompatVisitorFactory :
    AsmClassVisitorFactory<InstrumentationParameters.None> {
    override fun isInstrumentable(classData: ClassData): Boolean =
        classData.className == jacksonExceptionUtil ||
            classData.className in jacksonApi24ParameterCountCalls

    override fun createClassVisitor(
        classContext: ClassContext,
        nextClassVisitor: ClassVisitor,
    ): ClassVisitor {
        val className = classContext.currentClassData.className
        val expectedRewrites = jacksonApi24ParameterCountCalls[className] ?: 1
        return JacksonApi24CompatClassVisitor(nextClassVisitor, className, expectedRewrites)
    }
}

private class JacksonApi24CompatClassVisitor(
    nextClassVisitor: ClassVisitor,
    private val className: String,
    private val expectedRewrites: Int,
) : ClassVisitor(Opcodes.ASM9, nextClassVisitor) {
    private var rewrites = 0

    override fun visitMethod(
        access: Int,
        name: String,
        descriptor: String,
        signature: String?,
        exceptions: Array<out String>?,
    ): MethodVisitor {
        val next = super.visitMethod(access, name, descriptor, signature, exceptions)
        return object : MethodVisitor(Opcodes.ASM9, next) {
            override fun visitTypeInsn(opcode: Int, type: String) {
                if (
                    className == jacksonExceptionUtil &&
                    name == "isFatal" &&
                    descriptor == "(Ljava/lang/Throwable;)Z" &&
                    opcode == Opcodes.INSTANCEOF &&
                    type == bootstrapMethodError
                ) {
                    super.visitMethodInsn(
                        Opcodes.INVOKESTATIC,
                        api24CompatHelper,
                        "isBootstrapMethodError",
                        "(Ljava/lang/Throwable;)Z",
                        false,
                    )
                    rewrites += 1
                    return
                }
                super.visitTypeInsn(opcode, type)
            }

            override fun visitMethodInsn(
                opcode: Int,
                owner: String,
                methodName: String,
                methodDescriptor: String,
                isInterface: Boolean,
            ) {
                if (
                    className != jacksonExceptionUtil &&
                    opcode == Opcodes.INVOKEVIRTUAL &&
                    owner in api24UnsafeReflectionOwners &&
                    methodName == "getParameterCount" &&
                    methodDescriptor == "()I"
                ) {
                    super.visitMethodInsn(
                        Opcodes.INVOKEVIRTUAL,
                        owner,
                        "getParameterTypes",
                        "()[Ljava/lang/Class;",
                        false,
                    )
                    super.visitInsn(Opcodes.ARRAYLENGTH)
                    rewrites += 1
                    return
                }
                super.visitMethodInsn(opcode, owner, methodName, methodDescriptor, isInterface)
            }
        }
    }

    override fun visitEnd() {
        check(rewrites == expectedRewrites) {
            "Jackson API 24 compatibility transform expected $expectedRewrites rewrite(s) " +
                "in $className but found $rewrites; review the Tauri/Jackson dependency update"
        }
        super.visitEnd()
    }
}
