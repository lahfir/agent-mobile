package com.lahfir.agentmobile.driver

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.Bundle
import android.os.Process

internal fun isAuthorizedCaller(uid: Int): Boolean =
    uid == Process.ROOT_UID || uid == Process.SHELL_UID

class ProvisionProvider : ContentProvider() {

    override fun onCreate(): Boolean = true

    override fun call(authority: String, method: String, arg: String?, extras: Bundle?): Bundle? =
        handleCall(method)

    override fun call(method: String, arg: String?, extras: Bundle?): Bundle? =
        handleCall(method)

    private fun handleCall(method: String): Bundle {
        if (!isAuthorizedCaller(Binder.getCallingUid())) {
            throw SecurityException("caller not permitted")
        }
        if (method != METHOD_PROVISION) {
            throw IllegalArgumentException("unsupported method")
        }
        val context = context ?: throw IllegalStateException("provider not attached")
        val token = TokenStore.generateToken()
        TokenStore(context).replace(token)
        AgentMobileAccessibilityService.activeInstance?.onTokenRotated()
        return Bundle().apply { putString(RESULT_TOKEN, token) }
    }

    override fun getType(uri: Uri): String = throw UnsupportedOperationException()

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor = throw UnsupportedOperationException()

    override fun insert(uri: Uri, values: ContentValues?): Uri = throw UnsupportedOperationException()

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int =
        throw UnsupportedOperationException()

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = throw UnsupportedOperationException()

    companion object {
        const val METHOD_PROVISION = "provision"
        const val RESULT_TOKEN = "token"
    }
}
